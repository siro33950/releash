use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn serve(response: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/image", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 4096];
        stream.read(&mut buffer).await.unwrap();
        // 上限を超えた応答では受信側が接続を閉じる。
        let _ = stream.write_all(&response).await;
    });
    (url, task)
}

#[tokio::test]
async fn test_comment画像_本文と媒体型を取得して返す() {
    // Given
    let (url, server) = serve(b"HTTP/1.1 200 OK\r\nContent-Type: image/png; charset=binary\r\nContent-Length: 4\r\n\r\npng!".to_vec()).await;
    // When
    let image = HttpCommentImageQuery::default().image(&url).await.unwrap();
    // Then
    assert_eq!(image.data, b"png!");
    assert_eq!(image.media_type, "image/png");
    server.await.unwrap();
}

#[tokio::test]
async fn test_comment画像_不正urlと認証情報と外部取得以外のschemeを拒否する() {
    // Given
    let query = HttpCommentImageQuery::default();
    for url in [
        "invalid",
        "file:///etc/passwd",
        "data:image/png;base64,AA==",
        "http://user:password@localhost/image",
    ] {
        // When / Then
        assert!(query.image(url).await.is_err(), "{url}");
    }
}

#[tokio::test]
async fn test_comment画像_httpエラーと非画像と過大なcontent_lengthを返さない() {
    for headers in [
        "HTTP/1.1 404 Not Found\r\nContent-Type: image/png\r\nContent-Length: 0",
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0",
        "HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: 0",
        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 8388609",
    ] {
        // Given
        let (url, server) = serve(format!("{headers}\r\n\r\n").into_bytes()).await;
        // When / Then
        assert!(HttpCommentImageQuery::default().image(&url).await.is_err());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn test_comment画像_content_lengthがなくても受信量の上限を守る() {
    // Given
    let mut response =
        b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nConnection: close\r\n\r\n".to_vec();
    response.resize(response.len() + MAX_IMAGE_BYTES + 1, 1);
    let (url, server) = serve(response).await;
    // When
    let result = HttpCommentImageQuery::default().image(&url).await;
    // Then
    assert!(result.unwrap_err().message.contains("8 MiB"));
    server.await.unwrap();
}
