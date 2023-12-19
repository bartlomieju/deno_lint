// Copyright 2020-2021 the Deno authors. All rights reserved. MIT license.
use crate::ast_parser::parse_program;
use crate::context::Context;
use crate::control_flow::ControlFlow;
use crate::diagnostic::LintDiagnostic;
use crate::ignore_directives::{
  parse_file_ignore_directives, parse_line_ignore_directives,
};
use crate::rules::{ban_unknown_rule_code::BanUnknownRuleCode, LintRule};
use deno_ast::Diagnostic;
use deno_ast::MediaType;
use deno_ast::ParsedSource;
use deno_ast::Scope;

use std::time::Instant;

// TODO(bartlomieju): I'm not yet sure about Send and Sync here.
// Fine for now to get `dlint` compiling, but it should be optimized
// to spawn the fewest number of `JsRuntime` instances possible.
pub type LintPluginCallback = dyn Fn(&mut Context) + Send + Sync;

// #[derive(Clone)]
pub struct LintPlugin {
  name: String,
  callback: Box<LintPluginCallback>
}

impl std::fmt::Debug for LintPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LintPlugin").field("name", &self.name).finish()
    }
}

pub struct LinterBuilder {
  ignore_file_directive: String,
  ignore_diagnostic_directive: String,
  media_type: MediaType,
  rules: Vec<&'static dyn LintRule>,
  plugins: Vec<LintPlugin>,
}

impl Default for LinterBuilder {
  fn default() -> Self {
    Self {
      ignore_file_directive: "deno-lint-ignore-file".to_string(),
      ignore_diagnostic_directive: "deno-lint-ignore".to_string(),
      media_type: MediaType::TypeScript,
      rules: Vec::new(),
      plugins: Vec::new(),
    }
  }
}

impl LinterBuilder {
  pub fn build(self) -> Linter {
    Linter::new(
      self.ignore_file_directive,
      self.ignore_diagnostic_directive,
      self.media_type,
      self.rules,
      self.plugins,
    )
  }

  /// Set name for directive that can be used to skip linting file.
  ///
  /// Defaults to "deno-lint-ignore-file".
  pub fn ignore_file_directive(mut self, directive: &str) -> Self {
    self.ignore_file_directive = directive.to_owned();
    self
  }

  /// Set name for directive that can be used to ignore next line.
  ///
  /// Defaults to "deno-lint-ignore".
  pub fn ignore_diagnostic_directive(mut self, directive: &str) -> Self {
    self.ignore_diagnostic_directive = directive.to_owned();
    self
  }

  /// Set media type of a file to be linted.
  ///
  /// Defaults to `MediaType::TypeScript`
  pub fn media_type(mut self, media_type: MediaType) -> Self {
    self.media_type = media_type;
    self
  }

  /// Set a list of rules that will be used for linting.
  ///
  /// Defaults to empty list (no rules will be run by default).
  pub fn rules(mut self, rules: Vec<&'static dyn LintRule>) -> Self {
    self.rules = rules;
    self
  }

  /// Set a list of plugins that will be used for linting.
  pub fn plugins(mut self, plugins: Vec<LintPlugin>) -> Self {
    self.plugins = plugins;
    self
  }
}

pub struct Linter {
  ignore_file_directive: String,
  ignore_diagnostic_directive: String,
  media_type: MediaType,
  rules: Vec<&'static dyn LintRule>,
  plugins: Vec<LintPlugin>,
}

impl Linter {
  fn new(
    ignore_file_directive: String,
    ignore_diagnostic_directive: String,
    media_type: MediaType,
    rules: Vec<&'static dyn LintRule>,
    plugins: Vec<LintPlugin>,
  ) -> Self {
    Linter {
      ignore_file_directive,
      ignore_diagnostic_directive,
      media_type,
      rules,
      plugins,
    }
  }

  pub fn lint(
    mut self,
    file_name: String,
    source_code: String,
  ) -> Result<(ParsedSource, Vec<LintDiagnostic>), Diagnostic> {
    let start = Instant::now();

    let syntax = deno_ast::get_syntax(self.media_type);
    let parse_result = parse_program(&file_name, syntax, source_code);

    let end_parse_program = Instant::now();
    debug!(
      "ast_parser.parse_program took {:#?}",
      end_parse_program - start
    );

    let parsed_source = parse_result?;
    let diagnostics = self.lint_program(&parsed_source);

    let end = Instant::now();
    debug!("Linter::lint took {:#?}", end - start);
    Ok((parsed_source, diagnostics))
  }

  pub fn lint_with_ast(
    mut self,
    parsed_source: &ParsedSource,
  ) -> Vec<LintDiagnostic> {
    let start = Instant::now();
    let diagnostics = self.lint_program(parsed_source);
    let end = Instant::now();
    debug!("Linter::lint_with_ast took {:#?}", end - start);

    diagnostics
  }

  fn filter_diagnostics(&self, mut context: Context) -> Vec<LintDiagnostic> {
    let start = Instant::now();

    let mut filtered_diagnostics = context.check_ignore_directive_usage();
    // Run `ban-unknown-rule-code`
    filtered_diagnostics.extend(context.ban_unknown_rule_code());
    // Run `ban-unused-ignore`
    filtered_diagnostics.extend(context.ban_unused_ignore(&self.rules));
    filtered_diagnostics.sort_by_key(|d| d.range.start.line_index);

    let end = Instant::now();
    debug!("Linter::filter_diagnostics took {:#?}", end - start);

    filtered_diagnostics
  }

  fn lint_program(
    &mut self,
    parsed_source: &ParsedSource,
  ) -> Vec<LintDiagnostic> {
    let start = Instant::now();
    let check_unknown_rules = self
      .rules
      .iter()
      .any(|a| a.code() == (BanUnknownRuleCode).code());
    let control_flow = ControlFlow::analyze(parsed_source);
    let diagnostics = parsed_source.with_view(|pg| {
      let file_ignore_directive =
        parse_file_ignore_directives(&self.ignore_file_directive, pg);

      // If a global ignore directive that has no codes specified exists, we must skip linting on
      // this file.
      if matches!(file_ignore_directive, Some(ref file_ignore) if file_ignore.ignore_all())
      {
        return vec![];
      }

      let line_ignore_directives =
        parse_line_ignore_directives(&self.ignore_diagnostic_directive, pg);

      let scope = Scope::analyze(pg);

      let mut context = Context::new(
        parsed_source.clone(),
        self.media_type,
        pg,
        file_ignore_directive,
        line_ignore_directives,
        scope,
        control_flow,
        check_unknown_rules,
      );

      crate::rules::sort_rules_by_priority(&mut self.rules);

      // Run builtin rules
      for rule in self.rules.iter() {
        rule.lint_program_with_ast_view(&mut context, pg);
      }
      // for plugin in self.plugins.iter() {
      //   (plugin.callback)(&mut context);
      // }

      self.filter_diagnostics(context)
    });

    let end = Instant::now();
    debug!("Linter::lint_module took {:#?}", end - start);

    diagnostics
  }
}
