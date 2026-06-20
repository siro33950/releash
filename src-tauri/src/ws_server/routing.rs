use crate::protocol::*;

pub(super) async fn route_message(_msg: &WsMessage) -> Option<WsMessage> {
    Some(WsMessage::Error(ErrorMsg {
        code: "INVALID_MESSAGE".to_string(),
        message: "Unexpected message from client".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use crate::protocol::*;

    use super::route_message;

    #[tokio::test]
    async fn test_route_known_inbound_message_returns_error() {
        let msg = WsMessage::AuthChallenge(AuthChallenge {
            challenge: "x".to_string(),
        });
        let result = route_message(&msg).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "INVALID_MESSAGE"),
            _ => panic!("expected error"),
        }
    }
}
