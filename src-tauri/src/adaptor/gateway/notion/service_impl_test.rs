use super::*;
use crate::common::operation_context::OperationContext;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::test]
async fn test_notion再試行_429は二回で打ち切らず相手の指定で再送する() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for attempt in 0..5 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).unwrap() > 0);
            if attempt < 4 {
                socket.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nRetry-After: 0\r\nConnection: close\r\n\r\n").unwrap();
            } else {
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .unwrap();
            }
        }
    });
    let response = send_with_retry(
        &build_client("token").unwrap(),
        &url,
        &serde_json::json!({}),
        &crate::common::retry::RetryLimiter::deterministic(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    server.join().unwrap();
}

#[tokio::test]
async fn test_notion通信_応答body待ちを取り消せる() {
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
    let result = crate::common::operation_context::scope(context, async {
        send(build_client("token").unwrap().get(url)).await
    })
    .await;
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
    assert_eq!(
        crate::common::operation_context::spawn_blocking(move || server.join().unwrap())
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn test_notion再試行_retry_afterの待ちを取り消せる() {
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
    let result = crate::common::operation_context::scope(context, async {
        send_with_retry(
            &build_client("token").unwrap(),
            &url,
            &serde_json::json!({}),
            &crate::common::retry::RetryLimiter::deterministic(),
        )
        .await
    })
    .await;
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

#[tokio::test]
async fn test_notion通信_応答body待ちが引き継いだ期限で終わる() {
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
    let result = crate::common::operation_context::scope(context, async {
        send(build_client("token").unwrap().get(url)).await
    })
    .await;
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
    assert_eq!(
        crate::common::operation_context::spawn_blocking(move || server.join().unwrap())
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn test_notion再試行_retry_after待ちが引き継いだ期限で終わる() {
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
    let result = crate::common::operation_context::scope(context, async {
        send_with_retry(
            &build_client("token").unwrap(),
            &url,
            &serde_json::json!({}),
            &crate::common::retry::RetryLimiter::deterministic(),
        )
        .await
    })
    .await;
    // Then
    assert!(matches!(result, Err(NotionError::ApiError(message)) if message.contains("HTTP 429")));
    assert!(start.elapsed() < Duration::from_millis(500));
    let listener = server.join().unwrap();
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn test_notion資源期限_親が無期限でも長い期限でも十秒で終了する() {
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
        let result = crate::common::operation_context::scope(context, async {
            send(
                build_client("token")
                    .unwrap()
                    .get(url)
                    .timeout(Duration::from_secs(30)),
            )
            .await
        })
        .await;
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
        assert_eq!(
            crate::common::operation_context::spawn_blocking(move || server
                .join()
                .unwrap()
                .unwrap())
            .await
            .unwrap(),
            0
        );
    }
}

#[test]
fn test_retry_afterは秒数とhttp日時を扱い不正な値は指定なしになる() {
    let now = std::time::UNIX_EPOCH + Duration::from_secs(1445412480);
    assert_eq!(retry_after("12", now), Some(Duration::from_secs(12)));
    assert_eq!(
        retry_after("Wed, 21 Oct 2015 07:28:03 GMT", now),
        Some(Duration::from_secs(3))
    );
    assert_eq!(
        retry_after("Wed, 21 Oct 2015 07:27:59 GMT", now),
        Some(Duration::ZERO)
    );
    assert_eq!(retry_after("invalid", now), None);
    assert_eq!(retry_after("-1", now), None);
}

#[tokio::test]
async fn test_notion通信_429以外のhttp失敗はstatusと本文を保持する() {
    for status in [401, 403, 500] {
        // Given
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let body = format!("{{\"object\":\"error\",\"message\":\"cause-{status}\"}}");
        let expected = body.clone();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).unwrap() > 0);
            write!(
                socket,
                "HTTP/1.1 {status} Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        // When
        let result = send_with_retry(
            &build_client("token").unwrap(),
            &url,
            &serde_json::json!({}),
            &crate::common::retry::RetryLimiter::deterministic(),
        )
        .await;
        // Then
        let Err(NotionError::ApiError(message)) = result else {
            panic!("HTTP failure must remain an API error")
        };
        assert!(message.contains(&format!("HTTP {status}")), "{message}");
        assert!(message.contains(&expected), "{message}");
        use crate::adaptor::presenter::connect::ConnectFailure;
        assert_eq!(
            NotionError::ApiError(message).connect_code(),
            connectrpc::ErrorCode::FailedPrecondition
        );
        server.join().unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn test_notion再試行_予算が尽きた429は待たずに返す() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let limiter = Arc::new(crate::common::retry::RetryLimiter::deterministic());
    let budget = limiter.clone();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).unwrap() > 0);
        while budget.try_acquire() {}
        socket.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nRetry-After: 60\r\nConnection: close\r\n\r\n").unwrap();
        listener
    });
    let start = Instant::now();
    let client = build_client("token").unwrap();
    let body = serde_json::json!({});
    let request = send_with_retry(&client, &url, &body, &limiter);
    tokio::pin!(request);
    let result = loop {
        tokio::select! {
            result = &mut request => break result,
            _ = tokio::task::yield_now() => assert!(start.elapsed() < Duration::from_secs(2)),
        }
    };
    assert!(matches!(result, Err(NotionError::ApiError(message)) if message.contains("HTTP 429")));
    assert!(start.elapsed() < Duration::from_secs(1));
    let listener = server.join().unwrap();
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn test_notion再試行_retry_afterが無ければ一秒待つ() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut last = None;
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).unwrap() > 0);
            if attempt == 0 {
                last = Some(Instant::now());
                socket.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            } else {
                assert!(last.unwrap().elapsed() >= Duration::from_secs(1));
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .unwrap();
            }
        }
    });
    send_with_retry(
        &build_client("token").unwrap(),
        &url,
        &serde_json::json!({}),
        &crate::common::retry::RetryLimiter::deterministic(),
    )
    .await
    .unwrap();
    server.join().unwrap();
}
