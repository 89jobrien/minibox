use std::os::unix::io::AsRawFd;
use tokio::net::{UnixListener, UnixStream};
use nix::fcntl::{fcntl, FcntlArg, FdFlag};
use tempfile::tempdir;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let socket_path = dir.path().join("test.sock");
    
    let listener = UnixListener::bind(&socket_path)?;
    let raw_fd = listener.as_raw_fd();
    let flags = fcntl(raw_fd, FcntlArg::F_GETFD)?;
    let is_cloexec = FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC);
    println!("Listener FD_CLOEXEC: {}", is_cloexec);
    
    let connect_task = tokio::spawn(async move {
        let _stream = UnixStream::connect(&socket_path).await.unwrap();
    });
    
    let (stream, _addr) = listener.accept().await?;
    let stream_raw_fd = stream.as_raw_fd();
    let stream_flags = fcntl(stream_raw_fd, FcntlArg::F_GETFD)?;
    let stream_is_cloexec = FdFlag::from_bits_truncate(stream_flags).contains(FdFlag::FD_CLOEXEC);
    println!("Stream FD_CLOEXEC: {}", stream_is_cloexec);
    
    connect_task.await?;
    Ok(())
}
