use anyhow::Context;
use axum::{body::Bytes, extract::Extension};
use tracing::warn;
use webrtc_unreliable::{Server as RtcServer, SessionEndpoint};

pub async fn new_rtc_session(
    Extension(rtc_session_endpoint): Extension<SessionEndpoint>,
    data: Bytes,
) -> Result<String, String> {
    Ok(rtc_session_endpoint
        .clone()
        .session_request(futures::stream::once(futures::future::ok::<
            Vec<u8>,
            std::io::Error,
        >(data.to_vec())))
        .await
        .map_err(|e| e.to_string())?)
}

/// Launch and run the webrtc echo server.
///
/// Must be called from a tokio runtime context.
pub async fn launch_and_run_webrtc(
    public_webrtc_addr: std::net::SocketAddr,
    location_description: String,
) -> anyhow::Result<SessionEndpoint> {
    let webrtc_listen_addr = std::net::SocketAddr::new(
        std::net::Ipv4Addr::UNSPECIFIED.into(),
        public_webrtc_addr.port(),
    );

    let mut rtc_server = RtcServer::new(webrtc_listen_addr, public_webrtc_addr)
        .await
        .context("could not start RTC server")?;

    let session_endpoint = rtc_server.session_endpoint();

    tokio::spawn(async move {
        let mut buf = Vec::new();
        loop {
            let (message_type, remote_addr) = match rtc_server.recv().await {
                Ok(received) => {
                    metrics::increment_counter!("webrtc_pings_total");
                    buf.clear();
                    buf.extend(received.message.as_slice());
                    (received.message_type, received.remote_addr)
                }
                Err(err) => {
                    warn!("could not receive RTC message: {}", err);
                    continue;
                }
            };
            let data: &str = match std::str::from_utf8(&buf) {
                Ok(v) => v,
                Err(_) => continue,
            };
            // last line will get echo'ed back
            let mut last_line = "";
            let mut send_location = false;
            for line in data.split('\n') {
                if line.starts_with("LOC?") {
                    send_location = true;
                }
                last_line = line;
            }
            let mut to_send = last_line;
            // only allocate storage when sending the location since that is not very frequent
            let storage_when_sending_location: String;
            if send_location {
                storage_when_sending_location =
                    format!("LOC:\t{}\n{}", &location_description, last_line);
                to_send = &storage_when_sending_location;
            }
            let send_result = rtc_server
                .send(to_send.as_bytes(), message_type, &remote_addr)
                .await;
            if let Err(err) = send_result {
                warn!("could not send RTC message: {}", err)
            }
        }
    });

    Ok(session_endpoint)
}
