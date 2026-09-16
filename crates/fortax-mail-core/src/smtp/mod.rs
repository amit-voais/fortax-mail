//! SMTP sending via lettre, with password or XOAUTH2 auth.

use crate::error::{CoreError, Result};
use crate::models::AccountConfig;
use lettre::transport::smtp::{
    authentication::{Credentials, Mechanism},
    client::AsyncSmtpConnection,
    extension::ClientId,
};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use std::{
    fmt,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;

const SMTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub enum SmtpAuth {
    Password(String),
    XOAuth2(String),
}

fn build_transport(
    cfg: &AccountConfig,
    auth: &SmtpAuth,
) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    use lettre::transport::smtp::client::{Tls, TlsParameters};

    use crate::models::ConnectionSecurity;
    let implicit = match cfg.settings.connection.smtp_security {
        ConnectionSecurity::Tls => true,
        ConnectionSecurity::Starttls => false,
        ConnectionSecurity::Auto => cfg.smtp_port == 465,
    };
    let mut params = TlsParameters::builder(cfg.smtp_host.clone());
    if crate::imap::tls_insecure() && cfg.settings.connection.trusted_certificate_pem.is_empty() {
        params = params
            .dangerous_accept_invalid_certs(true)
            .dangerous_accept_invalid_hostnames(true);
    }
    let params = params.build().map_err(|e| CoreError::Tls(e.to_string()))?;
    let builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.smtp_host)
        .port(cfg.smtp_port)
        .tls(if implicit {
            Tls::Wrapper(params)
        } else {
            Tls::Required(params)
        });

    let builder = match auth {
        SmtpAuth::Password(pw) => builder
            .credentials(Credentials::new(cfg.username.clone(), pw.clone()))
            .authentication(vec![Mechanism::Plain, Mechanism::Login]),
        SmtpAuth::XOAuth2(token) => builder
            .credentials(Credentials::new(cfg.username.clone(), token.clone()))
            .authentication(vec![Mechanism::Xoauth2]),
    };

    // Bound greeting, STARTTLS and AUTH. Exchange Online can leave a throttled
    // SMTP session open without completing one of these phases; without a
    // transport timeout a queued Outlook send remains inflight forever.
    Ok(builder.timeout(Some(SMTP_TIMEOUT)).build())
}

/// A rustls stream that lettre can use through its public SMTP protocol
/// client. For STARTTLS, `greeting` supplies the greeting that lettre expects
/// before its post-upgrade EHLO; the real greeting was consumed while
/// negotiating STARTTLS.
struct VerifiedSmtpStream {
    inner: tokio_rustls::client::TlsStream<TcpStream>,
    greeting: &'static [u8],
}

impl fmt::Debug for VerifiedSmtpStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("VerifiedSmtpStream").finish()
    }
}

impl AsyncRead for VerifiedSmtpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if !self.greeting.is_empty() && buffer.remaining() != 0 {
            let length = self.greeting.len().min(buffer.remaining());
            buffer.put_slice(&self.greeting[..length]);
            self.greeting = &self.greeting[length..];
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(context, buffer)
    }
}

impl AsyncWrite for VerifiedSmtpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(context)
    }
}

impl lettre::transport::smtp::client::AsyncTokioStream for VerifiedSmtpStream {
    fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        self.inner.get_ref().0.peer_addr()
    }
}

async fn read_smtp_response(stream: &mut TcpStream, stage: &str) -> Result<String> {
    const MAX_RESPONSE_BYTES: usize = 100_000;
    const MAX_LINE_BYTES: usize = 1_000;

    let mut response = Vec::new();
    loop {
        let line_start = response.len();
        loop {
            if response.len() >= MAX_RESPONSE_BYTES
                || response.len().saturating_sub(line_start) >= MAX_LINE_BYTES
            {
                return Err(CoreError::Smtp(format!(
                    "SMTP {stage} response was too large"
                )));
            }
            let mut byte = [0];
            stream
                .read_exact(&mut byte)
                .await
                .map_err(|error| CoreError::Smtp(format!("SMTP {stage}: {error}")))?;
            response.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }

        let line = &response[line_start..];
        if line.len() < 4 || !line[..3].iter().all(u8::is_ascii_digit) {
            return Err(CoreError::Smtp(format!(
                "SMTP {stage} returned an invalid response"
            )));
        }
        if line[3] == b' ' {
            return String::from_utf8(response)
                .map_err(|_| CoreError::Smtp(format!("SMTP {stage} was not valid UTF-8")));
        }
        if line[3] != b'-' {
            return Err(CoreError::Smtp(format!(
                "SMTP {stage} returned an invalid response"
            )));
        }
    }
}

fn expect_smtp_status(response: &str, expected: &str, stage: &str) -> Result<()> {
    if response.as_bytes().get(..3) == Some(expected.as_bytes()) {
        Ok(())
    } else {
        Err(CoreError::Smtp(format!(
            "SMTP {stage} failed: {}",
            response.trim_end()
        )))
    }
}

fn map_smtp_error(cfg: &AccountConfig, error: lettre::transport::smtp::Error) -> CoreError {
    let message = error.to_string();
    if error.is_tls() {
        CoreError::Tls(format!(
            "SMTP {}:{}: {message}. Check SSL/TLS versus STARTTLS and the server certificate. For Proton Bridge, import its exported certificate.",
            cfg.smtp_host, cfg.smtp_port
        ))
    } else if message.contains("535") || message.to_ascii_lowercase().contains("auth") {
        CoreError::Auth(format!("smtp auth: {message}"))
    } else {
        CoreError::Smtp(format!("{}:{}: {message}", cfg.smtp_host, cfg.smtp_port))
    }
}

fn smtp_auth(auth: &SmtpAuth, username: &str) -> (Credentials, Vec<Mechanism>) {
    match auth {
        SmtpAuth::Password(password) => (
            Credentials::new(username.to_owned(), password.clone()),
            vec![Mechanism::Plain, Mechanism::Login],
        ),
        SmtpAuth::XOAuth2(token) => (
            Credentials::new(username.to_owned(), token.clone()),
            vec![Mechanism::Xoauth2],
        ),
    }
}

async fn imported_certificate_connection(
    cfg: &AccountConfig,
    auth: &SmtpAuth,
) -> Result<AsyncSmtpConnection> {
    use crate::models::ConnectionSecurity;

    let connect = async {
        let mut tcp = TcpStream::connect((&*cfg.smtp_host, cfg.smtp_port))
            .await
            .map_err(|error| {
                CoreError::Smtp(format!("{}:{}: {error}", cfg.smtp_host, cfg.smtp_port))
            })?;
        tcp.set_nodelay(true).ok();

        let starttls = match cfg.settings.connection.smtp_security {
            ConnectionSecurity::Tls => false,
            ConnectionSecurity::Starttls => true,
            ConnectionSecurity::Auto => cfg.smtp_port != 465,
        };
        if starttls {
            let greeting = read_smtp_response(&mut tcp, "greeting").await?;
            expect_smtp_status(&greeting, "220", "greeting")?;
            tcp.write_all(b"EHLO fortax.local\r\n")
                .await
                .map_err(|error| CoreError::Smtp(format!("SMTP EHLO: {error}")))?;
            let capabilities = read_smtp_response(&mut tcp, "EHLO").await?;
            expect_smtp_status(&capabilities, "250", "EHLO")?;
            if !capabilities.lines().any(|line| {
                line.get(4..)
                    .and_then(|text| text.split_ascii_whitespace().next())
                    .is_some_and(|capability| capability.eq_ignore_ascii_case("STARTTLS"))
            }) {
                return Err(CoreError::Tls(format!(
                    "SMTP {}:{} does not advertise STARTTLS",
                    cfg.smtp_host, cfg.smtp_port
                )));
            }
            tcp.write_all(b"STARTTLS\r\n")
                .await
                .map_err(|error| CoreError::Smtp(format!("SMTP STARTTLS: {error}")))?;
            let response = read_smtp_response(&mut tcp, "STARTTLS").await?;
            expect_smtp_status(&response, "220", "STARTTLS")?;
        }

        let server_name = rustls::pki_types::ServerName::try_from(cfg.smtp_host.clone())
            .map_err(|_| CoreError::Tls(format!("invalid SMTP hostname: {}", cfg.smtp_host)))?;
        let tls =
            crate::imap::account_tls_connector(&cfg.settings.connection.trusted_certificate_pem)?
                .connect(server_name, tcp)
                .await
                .map_err(|error| {
                    CoreError::Tls(format!("SMTP {}:{}: {error}", cfg.smtp_host, cfg.smtp_port))
                })?;
        let greeting = if starttls {
            b"220 Fortax STARTTLS ready\r\n".as_slice()
        } else {
            b"".as_slice()
        };
        let mut connection = AsyncSmtpConnection::connect_with_transport(
            Box::new(VerifiedSmtpStream {
                inner: tls,
                greeting,
            }),
            &ClientId::default(),
        )
        .await
        .map_err(|error| map_smtp_error(cfg, error))?;
        let (credentials, mechanisms) = smtp_auth(auth, &cfg.username);
        connection
            .auth(&mechanisms, &credentials)
            .await
            .map_err(|error| map_smtp_error(cfg, error))?;
        Ok(connection)
    };

    tokio::time::timeout(SMTP_TIMEOUT, connect)
        .await
        .map_err(|_| {
            CoreError::Smtp(format!(
                "SMTP {}:{} connection timed out after {}s",
                cfg.smtp_host,
                cfg.smtp_port,
                SMTP_TIMEOUT.as_secs()
            ))
        })?
}

/// Send a fully built RFC 5322 message.
pub async fn send_raw(
    cfg: &AccountConfig,
    auth: &SmtpAuth,
    from: &str,
    recipients: &[String],
    raw: &[u8],
) -> Result<()> {
    use lettre::address::Envelope;
    let from_addr = from
        .parse()
        .map_err(|e| CoreError::Smtp(format!("bad from address: {e}")))?;
    let mut tos = Vec::with_capacity(recipients.len());
    for r in recipients {
        tos.push(
            r.parse()
                .map_err(|e| CoreError::Smtp(format!("bad recipient {r}: {e}")))?,
        );
    }
    let envelope = Envelope::new(Some(from_addr), tos)
        .map_err(|e| CoreError::Smtp(format!("Invalid message envelope: {e}")))?;

    tracing::debug!(
        host = %cfg.smtp_host,
        port = cfg.smtp_port,
        auth = match auth {
            SmtpAuth::Password(_) => "password",
            SmtpAuth::XOAuth2(_) => "xoauth2",
        },
        bytes = raw.len(),
        recipients = recipients.len(),
        "smtp send_raw: connecting to relay",
    );
    let message = crate::mail_security::without_bcc(raw)?;
    if cfg.settings.connection.trusted_certificate_pem.is_empty() {
        build_transport(cfg, auth)?
            .send_raw(&envelope, &message)
            .await
            .map_err(|error| map_smtp_error(cfg, error))?;
    } else {
        let mut connection = imported_certificate_connection(cfg, auth).await?;
        connection
            .send(&envelope, &message)
            .await
            .map_err(|error| map_smtp_error(cfg, error))?;
        connection.abort().await;
    }
    Ok(())
}

/// Cheap connectivity/auth probe used by test_connection.
pub async fn test_connection(cfg: &AccountConfig, auth: &SmtpAuth) -> Result<()> {
    let ok = if cfg.settings.connection.trusted_certificate_pem.is_empty() {
        build_transport(cfg, auth)?
            .test_connection()
            .await
            .map_err(|error| map_smtp_error(cfg, error))?
    } else {
        use lettre::transport::smtp::commands::Noop;
        let mut connection = imported_certificate_connection(cfg, auth).await?;
        connection
            .command(Noop)
            .await
            .map_err(|error| map_smtp_error(cfg, error))?;
        connection.quit().await.ok();
        true
    };
    if ok {
        Ok(())
    } else {
        Err(CoreError::Smtp("connection test failed".into()))
    }
}
