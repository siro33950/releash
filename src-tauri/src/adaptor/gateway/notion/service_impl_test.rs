use super::*;
use crate::common::operation_context::OperationContext;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn test_notion通信_応答body待ちを取り消せる() {
    // Given
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let token = tokio_util::sync::CancellationToken::new();
    let signal = token.clone();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n")
            .unwrap();
        signal.cancel();
        socket.read(&mut request).unwrap()
    });
    let context = OperationContext::new(None, Arc::new(token));
    // When
    let result = crate::common::operation_context::sync_scope(context, || {
        send(build_client("token").unwrap().get(url))
    });
    // Then
    assert!(matches!(
        result,
        Err(NotionError::Technical(
            crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                ..
            }
        ))
    ));
    assert_eq!(server.join().unwrap(), 0);
}

#[test]
fn test_notion再試行_retry_afterの待ちを取り消せる() {
    // Given
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let token = tokio_util::sync::CancellationToken::new();
    let signal = token.clone();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).unwrap() > 0);
        socket.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nRetry-After: 60\r\nConnection: close\r\n\r\n").unwrap();
        std::thread::sleep(Duration::from_millis(50));
        signal.cancel();
    });
    let context = OperationContext::new(None, Arc::new(token));
    let start = Instant::now();
    // When
    let result = crate::common::operation_context::sync_scope(context, || {
        send_with_retry(
            &build_client("token").unwrap(),
            &url,
            &serde_json::json!({}),
        )
    });
    // Then
    assert!(matches!(
        result,
        Err(NotionError::Technical(
            crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                ..
            }
        ))
    ));
    assert!(start.elapsed() < Duration::from_secs(2));
    server.join().unwrap();
}

#[test]
fn test_notion通信_応答body待ちが引き継いだ期限で終わる() {
    use crate::common::operation_context::Deadline;
    // Given
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n")
            .unwrap();
        socket.read(&mut request).unwrap()
    });
    let start = Instant::now();
    let context = OperationContext::default()
        .with_deadline(Deadline::new(start + Duration::from_millis(500)));
    // When
    let result = crate::common::operation_context::sync_scope(context, || {
        send(build_client("token").unwrap().get(url))
    });
    // Then
    assert!(matches!(
        result,
        Err(NotionError::Technical(
            crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                ..
            }
        ))
    ));
    assert!(start.elapsed() < Duration::from_secs(3));
    assert_eq!(server.join().unwrap(), 0);
}

#[test]
fn test_notion再試行_retry_after待ちが引き継いだ期限で終わる() {
    use crate::common::operation_context::Deadline;
    // Given
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).unwrap() > 0);
        socket.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nRetry-After: 60\r\nConnection: close\r\n\r\n").unwrap();
        listener
    });
    let start = Instant::now();
    let context = OperationContext::default()
        .with_deadline(Deadline::new(start + Duration::from_millis(500)));
    // When
    let result = crate::common::operation_context::sync_scope(context, || {
        send_with_retry(
            &build_client("token").unwrap(),
            &url,
            &serde_json::json!({}),
        )
    });
    // Then
    assert!(matches!(
        result,
        Err(NotionError::Technical(
            crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                ..
            }
        ))
    ));
    assert!(start.elapsed() < Duration::from_secs(3));
    let listener = server.join().unwrap();
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn test_notion資源期限_親が無期限でも長い期限でも十秒で終了する() {
    use crate::common::operation_context::Deadline;
    for parent_deadline in [false, true] {
        // Given
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(15)))
                .unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n")
                .unwrap();
            socket.read(&mut request)
        });
        let start = Instant::now();
        let context = OperationContext::new(
            parent_deadline.then(|| Deadline::new(start + Duration::from_secs(30))),
            Arc::new(tokio_util::sync::CancellationToken::new()),
        );
        // When
        let result = crate::common::operation_context::sync_scope(context, || {
            send(
                build_client("token")
                    .unwrap()
                    .get(url)
                    .timeout(Duration::from_secs(30)),
            )
        });
        let elapsed = start.elapsed();
        // Then
        assert!(matches!(
            result,
            Err(NotionError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    ..
                }
            ))
        ));
        assert!(elapsed >= Duration::from_secs(10));
        assert!(elapsed < Duration::from_secs(15));
        assert_eq!(server.join().unwrap().unwrap(), 0);
    }
}
