use std::{
    io::{self, Write},
    process::Command,
};

fn main() {
    if cfg!(feature = "client") {
        println!("cargo::rerun-if-changed=client/src");
        println!("cargo::rerun-if-changed=client/static");

        let output = Command::new("npm")
            .args(["run", "build"])
            .current_dir("client")
            .output()
            .expect("failed to execute npm");

        io::stdout()
            .write_all(&output.stdout)
            .expect("failed to write to stdout");
        io::stderr()
            .write_all(&output.stderr)
            .expect("failed to write to stderr");

        assert!(output.status.success(), "npm failed");
    }
}
