//! Product-local installation support imported from xcsc.
#![deny(unsafe_op_in_unsafe_fn)]
#[cfg(not(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu")))]
compile_error!("installation support requires x86_64-unknown-linux-gnu");
pub mod contracts;
pub mod fs_safety;
pub mod schema_identity;
pub mod secret;
pub mod secret_envelope;
pub mod sqlite;
#[cfg(test)]
pub(crate) fn assert_one_subprocess_test(output: &std::process::Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success()
            && stdout.contains("running 1 test")
            && stdout.contains("1 passed;"),
        "expected one successful subprocess test: status={}, stdout={}, stderr={}",
        output.status,
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
}
