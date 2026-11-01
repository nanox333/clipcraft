use std::io::Write;
use std::process::{Command, Stdio};

fn run(input: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_clipcraft-cli"))
        .arg("bug-report")
        .env(
            "XDG_DATA_HOME",
            std::env::temp_dir().join("clipcraft-bug-report-cli-test"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn local_report_and_invalid_input() {
    let output = run(br#"{"observed":"Clipping stopped","steps":"Press F8"}"#);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Clipping stopped") && text.contains("Press F8"));
    assert!(!text.contains("Capture:"));
    let bad = run(br#"{"observed":""}"#);
    assert!(!bad.status.success());
    assert!(bad.stdout.is_empty());
}
