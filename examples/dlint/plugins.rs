// Copyright 2020-2021 the Deno authors. All rights reserved. MIT license.

use deno_ast::swc::common::BytePos;
use deno_ast::{ParsedSource, SourcePos};
use deno_core::{op2, OpState};
use deno_lint::diagnostic::{LintDiagnostic, Position, Range};
use std::rc::Rc;
use std::sync::mpsc::RecvError;
use std::sync::{Arc, Mutex};

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
  request_tx: std::sync::mpsc::Sender<PluginLintRequest>,
  response_rx: std::sync::mpsc::Receiver<PluginLintResponse>,
}

pub struct LintPluginHost {
  inner: Arc<Mutex<LintPluginHostInner>>,
}

impl LintPluginHost {
  pub fn lint(
    &self,
    filename: String,
    parsed_source: ParsedSource,
  ) -> Result<PluginLintResponse, RecvError> {
    let inner = self.inner.lock().unwrap();
    inner
      .request_tx
      .send(PluginLintRequest {
        filename,
        parsed_source,
      })
      .unwrap();
    inner.response_rx.recv()
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
  serde_json::json!({
      "filename": &ctx.filename,
      "ast": ctx.parsed_source.program_ref()
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
  esm_entry_point = "ext:dlint/plugin_server.js",
  esm = [
    dir "examples/dlint/runtime",
    "plugin_server.js"
  ],
);

pub fn create_plugin_host(plugins: Vec<String>) -> LintPluginHost {
  let (request_tx, request_rx) =
    std::sync::mpsc::channel::<PluginLintRequest>();
  let (response_tx, response_rx) =
    std::sync::mpsc::channel::<PluginLintResponse>();
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
  request_rx: std::sync::mpsc::Receiver<PluginLintRequest>,
  response_tx: std::sync::mpsc::Sender<PluginLintResponse>,
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
  let init_src = format!("globalThis.serverInit({})", init_config);
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
    "[plugin server] runtime created, took {:?}",
    std::time::Instant::now() - start
  );
  let op_state = js_runtime.op_state();

  while let Ok(request) = request_rx.recv() {
    eprintln!("[plugin server] received request {}", request.filename);
    let start = std::time::Instant::now();
    {
      let mut state = op_state.borrow_mut();
      state.put(PluginCtx {
        parsed_source: request.parsed_source,
        filename: request.filename,
        diagnostics: vec![],
      });
    }
    let src = "globalThis.serverRequest()".to_string();
    js_runtime.execute_script("request.js", src.into()).unwrap();
    let ctx = op_state.borrow_mut().take::<PluginCtx>();
    response_tx
      .send(PluginLintResponse {
        diagnostics: ctx.diagnostics,
      })
      .unwrap();
    eprintln!(
      "[plugin server] sent response {:?}",
      std::time::Instant::now() - start
    );
  }
}
