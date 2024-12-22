use lettre::address::AddressError;
use lettre::message::MultiPart;
use lettre::{transport::smtp::authentication::Credentials, SmtpTransport};
use lettre::{Message, Transport};
use minijinja::context;
use minijinja_autoreload::AutoReloader;
use thiserror::Error;
use tracing::info;

pub struct Mailer {
    pub mailer: SmtpTransport,
}

#[derive(Error, Debug)]
pub enum EmailError {
    #[error("Email construction error: {0}")]
    EmailParsingError(#[from] AddressError),

    #[error("Email sending error: {0}")]
    SendingError(#[from] lettre::transport::smtp::Error),

    #[error("Body constructing error: {0}")]
    BodyError(#[from] lettre::error::Error),

    #[error("Templating error: {0}")]
    TemplatingError(#[from] minijinja::Error),
}

impl Mailer {
    pub fn new(
        username: String,
        relayer: String,
        password: String,
    ) -> Result<Self, lettre::transport::smtp::Error> {
        let creds = Credentials::new(username, password);

        let mailer = SmtpTransport::relay(&relayer)?.credentials(creds).build();

        Ok(Mailer { mailer })
    }

    pub fn send_email_verification(
        &self,
        email_to: &str,
        verification_code: &str,
        templates: &AutoReloader,
    ) -> Result<(), EmailError> {
        let name = email_to
            .split('@')
            .next()
            .unwrap_or("User")
            .chars()
            .enumerate()
            .map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c })
            .collect::<String>();

        let context = context! {
            name => name,
            verification_code => verification_code
        };

        let env = templates.acquire_env()?;
        let html = env
            .get_template("email/email_verification.html")?
            .render(&context)?;
        let plain = env
            .get_template("email/email_verification.txt")?
            .render(&context)?;

        let email = Message::builder()
            .from("Admin <minne@starks.cloud>".parse()?)
            .reply_to("Admin <minne@starks.cloud>".parse()?)
            .to(format!("{} <{}>", name, email_to).parse()?)
            .subject("Verify Your Email Address")
            .multipart(MultiPart::alternative_plain_html(plain, html))?;

        info!("Sending email to: {}", email_to);
        self.mailer.send(&email)?;
        Ok(())
    }
}
