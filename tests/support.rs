use wiremock::MockServer;

pub async fn start_mock_server_if_available() -> Option<MockServer> {
    if std::net::TcpListener::bind(("127.0.0.1", 0)).is_err() {
        eprintln!("skipping wiremock test: local TCP sockets are unavailable in this environment");
        return None;
    }

    Some(MockServer::start().await)
}
