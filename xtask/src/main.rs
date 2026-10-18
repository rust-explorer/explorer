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

const DOCKER_BUILD: Step = Step {
    name: "docker build",
    program: "docker",
    args: &["build", "-t", "explorer:ci", "."],
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

    if command_exists("dx") {
        if !run(&WEB_BUILD) {
            return ExitCode::FAILURE;
        }
    } else {
        eprintln!(
            "\n==> skipping {}: `dx` not found (cargo binstall dioxus-cli@0.7.10)",
            WEB_BUILD.name
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
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}
