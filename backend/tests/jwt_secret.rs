use std::process::Command;

#[test]
fn fails_without_jwt_secret() {
    let exe = env!("CARGO_BIN_EXE_backend");
    let output = Command::new(exe)
        .env_remove("JWT_SECRET")
        .output()
        .expect("failed to run backend binary");
    assert!(!output.status.success());
}

