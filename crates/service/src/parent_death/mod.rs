use std::io;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub fn configure_command(command: &mut tokio::process::Command) -> io::Result<()> {
    linux::configure_command(command);
    Ok(())
}

#[cfg(windows)]
pub fn configure_command(command: &mut tokio::process::Command) -> io::Result<()> {
    windows::configure_command(command)
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn configure_command(_command: &mut tokio::process::Command) -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn exit_if_parent_missing() {
    linux::exit_if_parent_missing();
}

#[cfg(not(target_os = "linux"))]
pub fn exit_if_parent_missing() {}
