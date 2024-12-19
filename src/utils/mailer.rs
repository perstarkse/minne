use std::env;

use lettre::{transport::smtp::authentication::Credentials, SmtpTransport};

pub struct Mailer {
    pub mailer: SmtpTransport,
}

impl Mailer {
    pub fn new() -> Self {
        let creds = Credentials::new(
            env::var("SMTP_USERNAME").unwrap().to_owned(),
            env::var("SMTP_PASSWORD").unwrap().to_owned(),
        );

        let mailer = SmtpTransport::relay(env::var("SMTP_RELAYER").unwrap().as_str())
            .unwrap()
            .credentials(creds)
            .build();

        Mailer { mailer }
    }
}
