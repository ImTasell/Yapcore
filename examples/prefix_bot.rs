//! Minimal Yapper prefix-command bot.
//!
//! Run with:
//!   YAPPER_BOT_TOKEN=yb_... cargo run --example prefix_bot
//!
//! In a server where the bot is present, send:
//!   !ping
//!
//! This example uses the documented long-poll event transport. It does not
//! require a public HTTP endpoint, gateway connection, or voice dependencies.

use yapcore::{Bot, Result};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut bot = Bot::builder()
        .token_from_env("YAPPER_BOT_TOKEN")?
        .prefixes(["!", "/"])
        .build()?;

    bot.command(
        "ping",
        "Check whether the bot is alive",
        |context| async move {
            context.reply("Pong from Yapcore!").await?;
            Ok(())
        },
    )?;

    bot.run().await
}
