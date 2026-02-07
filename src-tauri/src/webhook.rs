pub fn send_notification(webhook_url: &str, exit_code: i32) {
    let url = webhook_url.to_string();
    std::thread::spawn(move || {
        let message = format!("releash: コマンドが終了しました (exit code: {})", exit_code);

        let body = serde_json::json!({
            "text": message,
            "content": message
        })
        .to_string();

        match ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_bytes(body.as_bytes())
        {
            Ok(_) => {
                log::info!("Webhook送信成功");
            }
            Err(e) => {
                log::warn!("Webhook送信失敗: {}", e);
            }
        }
    });
}
