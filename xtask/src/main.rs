use std::env;
use std::process::{Command, ExitCode};

const USAGE: &str = "usage: cargo xtask ci [--no-docker]";

struct Step {
    name: &'static str,
    program: &'static str,
    args: &'static [&'static str],
}

const STEPS: &[Step] = &[
    Step {
        name: "fmt",
        program: "cargo",
        args: &["fmt", "--all", "--check"],
    },
    Step {
        name: "clippy",
        program: "cargo",
        args: &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    },
    Step {
        name: "test",
        program: "cargo",
        args: &["test", "--workspace"],
    },
];

const WEB_BUILD: Step = Step {
    name: "web build",
    program: "dx",
    args: &[
        "build",
        "--release",
        "-p",
        "explorer-web",
        "--platform",
        "web",
    ],
};

const WEB_CLIPPY: Step = Step {
    name: "web clippy",
    program: "cargo",
    args: &[
        "clippy",
        "-p",
        "explorer-web",
        "--target",
        "wasm32-unknown-unknown",
        "--",
        "-D",
        "warnings",
    ],
};

const AUDIT: Step = Step {
    name: "audit",
    program: "cargo",
    args: &["audit"],
};

// Retried when `AUDIT` fails outside CI, so a failed advisory database fetch (offline)
// falls back to the local copy; a real advisory fails both runs.
const AUDIT_OFFLINE: Step = Step {
    name: "audit (local advisory database)",
    program: "cargo",
    args: &["audit", "--no-fetch", "--stale"],
};

const DOCKER_BUILD: Step = Step {
    name: "docker build",
    program: "docker",
    args: &[
        "build",
        "--platform",
        "linux/amd64",
        "-t",
        "explorer:ci",
        ".",
    ],
};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["ci"] => ci(true),
        ["ci", "--no-docker"] => ci(false),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn ci(docker: bool) -> ExitCode {
    for step in STEPS {
        if !run(step) {
            return ExitCode::FAILURE;
        }
    }

    if !audit() {
        return ExitCode::FAILURE;
    }

    if command_exists("dx") {
        for step in [&WEB_BUILD, &WEB_CLIPPY] {
            if !run(step) {
                return ExitCode::FAILURE;
            }
        }
    } else if env::var("CI").is_ok_and(|v| v == "true") {
        eprintln!(
            "\n==> FAILED: {}: `dx` not found but CI=true (cargo binstall dioxus-cli@0.7.10)",
            WEB_BUILD.name
        );
        return ExitCode::FAILURE;
    } else {
        eprintln!(
            "\n==> skipping {} and {}: `dx` not found (cargo binstall dioxus-cli@0.7.10)",
            WEB_BUILD.name, WEB_CLIPPY.name
        );
    }

    if docker {
        if !run(&DOCKER_BUILD) {
            return ExitCode::FAILURE;
        }
    } else {
        eprintln!("\n==> skipping {}: --no-docker", DOCKER_BUILD.name);
    }

    eprintln!("\n==> ci passed");
    ExitCode::SUCCESS
}

fn audit() -> bool {
    let ci = env::var("CI").is_ok_and(|v| v == "true");
    if !command_exists_with("cargo", &["audit", "--version"]) {
        if ci {
            eprintln!(
                "\n==> FAILED: {}: `cargo audit` not found but CI=true (cargo binstall cargo-audit)",
                AUDIT.name
            );
            return false;
        }
        eprintln!(
            "\n==> skipping {}: `cargo audit` not found (cargo binstall cargo-audit)",
            AUDIT.name
        );
        return true;
    }
    if run(&AUDIT) {
        return true;
    }
    !ci && run(&AUDIT_OFFLINE)
}

fn run(step: &Step) -> bool {
    eprintln!(
        "\n==> {}: {} {}",
        step.name,
        step.program,
        step.args.join(" ")
    );
    match Command::new(step.program).args(step.args).status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("\n==> FAILED: {} ({status})", step.name);
            false
        }
        Err(err) => {
            eprintln!(
                "\n==> FAILED: {} (cannot run {}: {err})",
                step.name, step.program
            );
            false
        }
    }
}

fn command_exists(program: &str) -> bool {
    command_exists_with(program, &["--version"])
}

fn command_exists_with(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|out| out.status.success())
}
