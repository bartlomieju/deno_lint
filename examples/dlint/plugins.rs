// Copyright 2020-2021 the Deno authors. All rights reserved. MIT license.

use deno_ast::swc::common as swc_common;
use deno_ast::swc::common::BytePos;
use deno_ast::{ParsedSource, SourcePos};
use deno_core::{op2, OpState};
use deno_lint::diagnostic::{LintDiagnostic, Position, Range};
use std::rc::Rc;
use std::sync::mpsc::RecvError;
use deno_ast::view::Comments;
use std::sync::{Arc, Mutex};
use swc_estree_compat::babelify;
use swc_estree_compat::babelify::Babelify;

pub struct PluginLintRequest {
  pub filename: String,
  pub parsed_source: ParsedSource,
}

#[derive(Debug)]
pub struct PluginLintResponse {
  pub diagnostics: Vec<LintDiagnostic>,
}

struct LintPluginHostInner {
  join_handle: std::thread::JoinHandle<()>,
  request_tx: tokio::sync::mpsc::UnboundedSender<PluginLintRequest>,
  response_rx: tokio::sync::mpsc::UnboundedReceiver<PluginLintResponse>,
}

pub struct LintPluginHost {
  inner: Arc<Mutex<LintPluginHostInner>>,
}

impl LintPluginHost {
  pub fn lint(
    &self,
    filename: String,
    parsed_source: ParsedSource,
  ) -> Option<PluginLintResponse> {
    let mut inner = self.inner.lock().unwrap();
    inner
      .request_tx
      .send(PluginLintRequest {
        filename,
        parsed_source,
      })
      .unwrap();
    inner.response_rx.blocking_recv()
  }
}

struct PluginCtx {
  parsed_source: ParsedSource,
  filename: String,
  diagnostics: Vec<LintDiagnostic>,
}

#[op2]
#[serde]
fn op_get_ctx(state: &OpState) -> serde_json::Value {
  let ctx = state.borrow::<PluginCtx>();

  // Create an ESTree compatbile AST
  let estree_ast = {
    let cm = Arc::new(swc_common::SourceMap::new(
      swc_common::FilePathMapping::empty(),
    ));
    let fm = Arc::new(swc_common::SourceFile::new(
      swc_common::FileName::Anon,
      false,
      swc_common::FileName::Anon,
      ctx.parsed_source.text_info().text_str().to_string(),
      BytePos(0),
    ));
    // let comments = deno_ast::MultiThreadedComments;
    let babelify_ctx = babelify::Context {
      fm,
      cm,
      comments: swc_node_comments::SwcComments::default(),
    };
    let program = ctx.parsed_source.program_ref().clone();
    serde_json::to_value(program.babelify(&babelify_ctx)).unwrap()
  };

  serde_json::json!({
      "filename": &ctx.filename,
      "ast": estree_ast
  })
}

#[op2]
fn op_add_diagnostic(
  state: &mut OpState,
  #[string] code: String,
  #[string] message: String,
  #[string] hint: Option<String>,
  #[smi] start: u32,
  #[smi] end: u32,
) {
  let ctx = state.borrow_mut::<PluginCtx>();

  let text_str = ctx.parsed_source.text_info().text_str();
  if text_str.is_empty() {
    return;
  }
  if text_str.len() < end as usize {
    return;
  }

  let start_source_pos = SourcePos::unsafely_from_byte_pos(BytePos(start));
  let end_source_pos = SourcePos::unsafely_from_byte_pos(BytePos(end));
  let text_info = ctx.parsed_source.text_info();
  let range = Range {
    start: Position::new(
      start as usize,
      text_info.line_and_column_index(start_source_pos),
    ),
    end: Position::new(
      end as usize,
      text_info.line_and_column_index(end_source_pos),
    ),
  };

  let lint_diagnostic = LintDiagnostic {
    range,
    filename: ctx.filename.to_string(),
    code,
    message,
    hint,
  };
  ctx.diagnostics.push(lint_diagnostic);
}

deno_core::extension!(dlint,
  ops = [op_get_ctx, op_add_diagnostic],
  esm_entry_point = "ext:dlint/plugin_host.js",
  esm = [
    dir "examples/dlint/runtime",
    "plugin_host.js",
    "visitor.js"
  ],
);

pub fn create_plugin_host(plugins: Vec<String>) -> LintPluginHost {
  let (request_tx, request_rx) =
    tokio::sync::mpsc::unbounded_channel::<PluginLintRequest>();
  let (response_tx, response_rx) =
    tokio::sync::mpsc::unbounded_channel::<PluginLintResponse>();
  let join_handle = std::thread::spawn(move || {
    let rt = tokio::runtime::Builder::new_current_thread()
      .enable_io()
      .enable_time()
      // This limits the number of threads for blocking operations (like for
      // synchronous fs ops) or CPU bound tasks like when we run dprint in
      // parallel for deno fmt.
      // The default value is 512, which is an unhelpfully large thread pool. We
      // don't ever want to have more than a couple dozen threads.
      .max_blocking_threads(4)
      .build()
      .unwrap();

    rt.block_on(run_plugin_host(plugins, request_rx, response_tx));
  });

  let inner = LintPluginHostInner {
    join_handle,
    request_tx,
    response_rx,
  };
  LintPluginHost {
    inner: Arc::new(Mutex::new(inner)),
  }
}

async fn run_plugin_host(
  plugins: Vec<String>,
  mut request_rx: tokio::sync::mpsc::UnboundedReceiver<PluginLintRequest>,
  response_tx: tokio::sync::mpsc::UnboundedSender<PluginLintResponse>,
) {
  let start = std::time::Instant::now();
  let mut js_runtime = deno_core::JsRuntime::new(deno_core::RuntimeOptions {
    extensions: vec![dlint::init_ops_and_esm()],
    module_loader: Some(Rc::new(deno_core::FsModuleLoader)),
    ..Default::default()
  });

  let init_config = serde_json::json!({
    "plugins": plugins
  });
  let init_src = format!("globalThis.hostInit({})", init_config);
  js_runtime
    .execute_script("init.js", init_src.into())
    .unwrap();
  js_runtime
    .run_event_loop(deno_core::PollEventLoopOptions {
      wait_for_inspector: false,
      pump_v8_message_loop: true,
    })
    .await
    .unwrap();
  eprintln!(
    "[plugin host] runtime created, took {:?}",
    std::time::Instant::now() - start
  );
  let op_state = js_runtime.op_state();

  while let Some(request) = request_rx.recv().await {
    eprintln!("[plugin host] received request {}", request.filename);
    let start = std::time::Instant::now();
    {
      let mut state = op_state.borrow_mut();
      state.put(PluginCtx {
        parsed_source: request.parsed_source,
        filename: request.filename,
        diagnostics: vec![],
      });
    }
    let src = "globalThis.hostRequest()".to_string();
    js_runtime.execute_script("request.js", src.into()).unwrap();
    let ctx = op_state.borrow_mut().take::<PluginCtx>();
    response_tx
      .send(PluginLintResponse {
        diagnostics: ctx.diagnostics,
      })
      .unwrap();
    eprintln!(
      "[plugin host] sent response {:?}",
      std::time::Instant::now() - start
    );
  }
}