use super::*;
use futures_util::stream;

fn client(messages: Vec<Value>) -> Client {
    Client::new(
        Box::pin(tokio::io::sink()),
        Box::pin(stream::iter(messages.into_iter().map(|value| {
            Ok(LogOutput::StdOut {
                message: format!("{value}\n").into(),
            })
        }))),
    )
}
fn response(id: u64, result: Value) -> Value {
    json!({"id":id,"result":result})
}
fn connected() -> Value {
    json!({"account":{"type":"chatgpt","email":"fixture@example.invalid","planType":"plus"},"requiresOpenaiAuth":true})
}
fn code() -> Value {
    json!({"type":"chatgptDeviceCode","loginId":"provider-id","verificationUrl":"https://auth.openai.com/codex/device","userCode":"ABCD-1234"})
}

#[tokio::test]
async fn completion_before_login_response_is_retained_then_account_is_confirmed() {
    let mut rpc = client(vec![
        response(1, json!({"userAgent":"codex-fixture"})),
        json!({"method":"account/login/completed","params":{"loginId":"provider-id","success":true}}),
        response(2, code()),
        response(3, connected()),
    ]);
    rpc.initialize().await.unwrap();
    let result = rpc
        .request("account/login/start", json!({"type":"chatgptDeviceCode"}))
        .await
        .unwrap();
    let (_, id) = challenge(result, Uuid::new_v4(), Utc::now()).unwrap();
    rpc.completion(&id).await.unwrap();
    let account = rpc.read_account().await.unwrap();
    assert_eq!(account.status, ConnectionStatus::Connected);
    assert_eq!(
        account.account.unwrap().email.as_deref(),
        Some("fixture@example.invalid")
    );
}

#[tokio::test]
async fn error_after_a_code_is_not_success_and_does_not_expose_provider_error() {
    let mut rpc = client(vec![
        response(1, code()),
        json!({"method":"account/login/completed",
        "params":{"loginId":"provider-id","success":false,"error":"secret-token-do-not-project"}}),
    ]);
    let result = rpc
        .request("account/login/start", json!({"type":"chatgptDeviceCode"}))
        .await
        .unwrap();
    let (_, id) = challenge(result, Uuid::new_v4(), Utc::now()).unwrap();
    assert_eq!(
        rpc.completion(&id).await,
        Err(AgentConnectionError::LoginFailed)
    );
}

#[tokio::test]
async fn cancel_and_logout_validate_response_and_observe_empty_account() {
    let mut rpc = client(vec![
        response(1, json!({"status":"canceled"})),
        response(2, json!({})),
        response(3, json!({"account":null,"requiresOpenaiAuth":true})),
    ]);
    rpc.cancel("provider-id").await.unwrap();
    assert_eq!(
        rpc.logout().await.unwrap().status,
        ConnectionStatus::Disconnected
    );
    let mut invalid = client(vec![response(1, json!({"status":"unexpected"}))]);
    assert_eq!(
        invalid.cancel("provider-id").await,
        Err(AgentConnectionError::Protocol)
    );
}

#[tokio::test]
async fn wrong_ids_and_provider_errors_fail_closed() {
    let mut rpc = client(vec![response(99, connected())]);
    assert!(matches!(
        rpc.read_account().await,
        Err(AgentConnectionError::Protocol)
    ));
    let mut rpc = client(vec![
        json!({"id":1,"error":{"message":"sensitive provider value"}}),
    ]);
    assert!(matches!(
        rpc.read_account().await,
        Err(AgentConnectionError::LoginFailed)
    ));
}

#[test]
fn malicious_urls_codes_depth_and_account_methods_are_rejected() {
    for url in [
        "javascript:alert(1)",
        "https://auth.openai.com.evil.invalid/codex/device",
        "https://user@auth.openai.com/codex/device",
        "https://auth.openai.com:8443/codex/device",
        "https://auth.openai.com/codex/device?token=secret",
    ] {
        let mut value = code();
        value["verificationUrl"] = json!(url);
        assert!(matches!(
            challenge(value, Uuid::new_v4(), Utc::now()),
            Err(AgentConnectionError::Protocol)
        ));
    }
    let mut value = code();
    value["userCode"] = json!("bad\ncode");
    assert!(challenge(value, Uuid::new_v4(), Utc::now()).is_err());
    for kind in ["apiKey", "amazonBedrock"] {
        assert!(matches!(
            account(json!({"account":{"type":kind},"requiresOpenaiAuth":true})),
            Err(AgentConnectionError::UnsupportedMethod)
        ));
    }
    let deep = format!("{{\"nested\":{}0{}}}", "[".repeat(40), "]".repeat(40));
    assert_eq!(
        parse_frame(deep.as_bytes()),
        Err(AgentConnectionError::Protocol)
    );
    assert_eq!(
        parse_frame(&vec![b' '; MAX_FRAME + 1]),
        Err(AgentConnectionError::Protocol)
    );
    assert!(matches!(
        account(
            json!({"account":{"type":"chatgpt","email":"x","planType":null},"requiresOpenaiAuth":true})
        ),
        Err(AgentConnectionError::Protocol)
    ));
}

#[tokio::test]
async fn fragmented_frames_and_stderr_never_become_account_data() {
    let raw = format!("{}\n", response(1, connected()));
    let pieces = vec![
        Ok(LogOutput::StdErr {
            message: "private-error\n".into(),
        }),
        Ok(LogOutput::StdOut {
            message: raw[..10].to_string().into(),
        }),
        Ok(LogOutput::StdOut {
            message: raw[10..].to_string().into(),
        }),
    ];
    let mut rpc = Client::new(Box::pin(tokio::io::sink()), Box::pin(stream::iter(pieces)));
    assert_eq!(
        rpc.read_account().await.unwrap().status,
        ConnectionStatus::Connected
    );
}

#[tokio::test]
async fn notification_and_stderr_floods_have_a_finite_read_budget() {
    let mut rpc = client(
        (0..129)
            .map(|_| json!({"method":"unrelated/notification","params":{}}))
            .collect(),
    );
    assert_eq!(
        rpc.completion("expected-login").await,
        Err(AgentConnectionError::Protocol)
    );
    let stderr = stream::iter((0..3).map(|_| {
        Ok(LogOutput::StdErr {
            message: vec![b'x'; 32 * 1024].into(),
        })
    }));
    let mut rpc = Client::new(Box::pin(tokio::io::sink()), Box::pin(stderr));
    assert!(matches!(
        rpc.read_account().await,
        Err(AgentConnectionError::Protocol)
    ));
}
