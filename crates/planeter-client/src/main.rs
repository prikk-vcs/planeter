//! The `planeter-client` program: real stdin, stdout and stderr; the language from `LC_ALL`, `LC_MESSAGES` or `LANG`
//! (English unless a *shipped* language is named; Japanese is a draft and is used only with `--lang ja`).
#![forbid(unsafe_code)]

use std::io::{IsTerminal, Write};

use planeter_client::catalog::Lang;
use planeter_client::{Ctx, http, run};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env = |name: &str| std::env::var(name).ok();
    let lang = Lang::from_environment(env);
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut out = std::io::stdout().lock();
    let mut err = std::io::stderr().lock();
    let interactive = std::io::stdin().is_terminal();
    let mut ctx = Ctx {
        lang,
        verbose: false,
        out: &mut out,
        err: &mut err,
        env: &env,
        stdin: &mut stdin,
        interactive,
        cwd: std::env::current_dir().unwrap_or_else(|_| ".".into()),
        prikk_program: std::env::var_os("PLANETER_CLIENT_PRIKK"),
        timeouts: http::Timeouts::default(),
    };
    let code = run(&args, &mut ctx);
    let _ = out.flush();
    let _ = err.flush();
    std::process::exit(code);
}
