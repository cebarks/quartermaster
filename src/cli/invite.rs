use anyhow::Result;

use super::common::CliContext;
use crate::invite::{generate_invite_code, parse_expiry};

pub fn run(expires: Option<&str>, ctx: &CliContext) -> Result<()> {
    let code = generate_invite_code();

    let expires_at = match expires {
        Some(exp) => Some(parse_expiry(exp)?),
        None => None,
    };

    ctx.db
        .create_invite(&code, None, expires_at.as_deref())
        .map_err(|e| anyhow::anyhow!("failed to create invite: {e}"))?;

    let display_host = if ctx.config.web_bind == "0.0.0.0" {
        "localhost"
    } else {
        &ctx.config.web_bind
    };

    println!("Invite code: {code}");
    println!(
        "Registration URL: http://{display_host}:{}/register?code={code}",
        ctx.config.web_port
    );

    if let Some(ref exp) = expires_at {
        println!("Expires: {exp}");
    } else {
        println!("Expires: never");
    }

    Ok(())
}
