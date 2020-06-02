// Copyright 2020 the Deno authors. All rights reserved. MIT license.
#[macro_use]
extern crate lazy_static;
use std::error::Error;

use clap::App;
use clap::Arg;

mod colors;
mod diagnostic;
mod linter;
mod rules;
mod scopes;
mod swc_util;

#[cfg(test)]
mod test_util;

fn create_cli_app<'a, 'b>() -> App<'a, 'b> {
  App::new("deno lint").arg(
    Arg::with_name("FILES")
      .help("Sets the input file to use")
      .required(true)
      .multiple(true),
  )
}

fn main() -> Result<(), Box<dyn Error>> {
  use linter::Linter;
  use rules::get_all_rules;

  #[cfg(windows)]
  colors::enable_ansi();

  let cli_app = create_cli_app();
  let matches = cli_app.get_matches();
  let file_names = matches.values_of("FILES").unwrap();

  let mut error_counts = 0;

  for file_name in file_names {
    let source_code = std::fs::read_to_string(&file_name)?;

    let mut linter = Linter::default();

    let rules = get_all_rules();

    let file_diagnostics =
      linter.lint(file_name.to_string(), source_code, rules)?;

    error_counts += file_diagnostics.len();
    for d in file_diagnostics.iter() {
      eprintln!("{}", d.to_pretty_string());
    }
  }

  if error_counts > 0 {
    eprintln!("Found {} problems", error_counts);
    std::process::exit(1);
  }

  Ok(())
}
