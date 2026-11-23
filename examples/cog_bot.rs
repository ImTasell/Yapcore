//! Modular bot example using a compiled cog, a command check and an error hook.
//!
//! Run with:
//!   YAPPER_BOT_TOKEN=yb_... cargo run --example cog_bot

use yapcore::{Bot, Cog, ErrorEvent};

struct UtilityCog;

impl Cog for UtilityCog {
    fn name(&self) -> &str {
        "utility"
    }

    fn register(&self, bot: &mut Bot) -> std::result::Result<(), String> {
        bot.command_with_check(
            "ping",
            "Check whether the bot is alive (requires a user ID)",
            |context| async move { context.author_id().is_some() },
            |context| async move {
                context.reply("Pong from Yapcore!").await?;
                Ok(())
            },
        )
        .map_err(|error| error.to_string())?;

        bot.on_event("member_join", |event| async move {
            println!("member_join event: {}", event.data);
            Ok(())
        })
        .map_err(|error| error.to_string())?;

        bot.button("utility:", |interaction| async move {
            interaction.ephemeral("Utility button received.").await?;
            Ok(())
        })
        .map_err(|error| error.to_string())?;

        Ok(())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut bot = Bot::builder()
        .token_from_env("YAPPER_BOT_TOKEN")?
        .logging(true)
        .build()?;

    bot.on_error(|error: ErrorEvent| async move {
        eprintln!("Yapcore error at {:?}: {}", error.stage, error.message);
        Ok(())
    });
    bot.add_cog(UtilityCog)?;

    bot.run().await?;
    Ok(())
}
