//! Statically compiled groups of commands, event listeners and button handlers.

use thiserror::Error;

use crate::bot::Bot;

/// Errors raised while validating or registering a cog.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum CogRegistrationError {
    /// Cog names must be 1–64 lowercase ASCII letters, numbers, `_` or `-`.
    #[error("cog names must be 1–64 lowercase ASCII letters, numbers, '_' or '-'")]
    InvalidName,
    /// A cog with this name has already been registered.
    #[error("cog '{0}' is already registered")]
    DuplicateName(String),
    /// Cog setup failed; all registrations performed by that cog were rolled back.
    #[error("cog '{name}' setup failed: {reason}")]
    SetupFailed {
        /// Cog name.
        name: String,
        /// Error supplied by the cog's registration method.
        reason: String,
    },
}

/// A statically compiled collection of bot handlers.
///
/// Implementations register their commands, event listeners and buttons in
/// `register`. A failed setup is rolled back by [`Bot::add_cog`].
///
/// Unlike Python cogs, Rust cogs are compiled into the application; they are
/// not discovered from `.py` files and cannot be hot-reloaded at runtime.
pub trait Cog: Send + Sync {
    /// Stable lowercase identifier for this cog.
    fn name(&self) -> &str;

    /// Register this cog's handlers on the supplied bot.
    ///
    /// Return an error string when any required registration fails. Yapcore
    /// restores the command, event and button registries when this returns an
    /// error, so a partially installed cog is never left behind.
    fn register(&self, bot: &mut Bot) -> std::result::Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BotBuilder;

    struct GoodCog;

    impl Cog for GoodCog {
        fn name(&self) -> &str {
            "welcome"
        }

        fn register(&self, bot: &mut Bot) -> std::result::Result<(), String> {
            bot.command("welcome", "Welcome a member", |_context| async { Ok(()) })
                .map_err(|error| error.to_string())?;
            bot.on_event("member_join", |_event| async { Ok(()) })
                .map_err(|error| error.to_string())?;
            bot.button("welcome:", |_context| async { Ok(()) })
                .map_err(|error| error.to_string())?;
            Ok(())
        }
    }

    struct FailingCog;

    impl Cog for FailingCog {
        fn name(&self) -> &str {
            "broken"
        }

        fn register(&self, bot: &mut Bot) -> std::result::Result<(), String> {
            bot.command("temporary", "Must be rolled back", |_context| async {
                Ok(())
            })
            .map_err(|error| error.to_string())?;
            bot.command("duplicate", "Conflicting command", |_context| async {
                Ok(())
            })
            .map_err(|error| error.to_string())?;
            Ok(())
        }
    }

    #[test]
    fn registers_cog_handlers_and_rejects_duplicate_cog_names() {
        let mut bot = BotBuilder::new().token("yb_test-token").build().unwrap();
        bot.add_cog(GoodCog).unwrap();
        assert_eq!(bot.cogs(), vec!["welcome"]);
        assert!(
            bot.command_definitions()
                .iter()
                .any(|item| item.name() == "welcome")
        );
        assert!(matches!(
            bot.add_cog(GoodCog),
            Err(CogRegistrationError::DuplicateName(name)) if name == "welcome"
        ));
    }

    #[test]
    fn restores_previous_cog_when_replacement_setup_fails() {
        struct BrokenReplacement;
        impl Cog for BrokenReplacement {
            fn name(&self) -> &str {
                "welcome"
            }
            fn register(&self, bot: &mut Bot) -> std::result::Result<(), String> {
                bot.command(
                    "greet",
                    "Partially added before failure",
                    |_context| async { Ok(()) },
                )
                .map_err(|error| error.to_string())?;
                Err("simulated setup failure".to_owned())
            }
        }

        let mut bot = BotBuilder::new().token("yb_test-token").build().unwrap();
        bot.add_cog(GoodCog).unwrap();
        let error = bot.replace_cog(BrokenReplacement).unwrap_err();
        assert!(
            matches!(error, CogRegistrationError::SetupFailed { name, .. } if name == "welcome")
        );
        assert_eq!(bot.cogs(), vec!["welcome"]);
        let names = bot
            .command_definitions()
            .into_iter()
            .map(|definition| definition.name().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, ["welcome"]);
    }

    #[test]
    fn removes_cog_registrations_as_a_unit() {
        let mut bot = BotBuilder::new().token("yb_test-token").build().unwrap();
        bot.add_cog(GoodCog).unwrap();
        assert!(bot.remove_cog("welcome"));
        assert!(bot.cogs().is_empty());
        assert!(bot.command_definitions().is_empty());
        assert!(!bot.remove_cog("welcome"));
    }

    #[test]
    fn replaces_cog_registration_without_reloading_compiled_code() {
        struct RevisedCog;
        impl Cog for RevisedCog {
            fn name(&self) -> &str {
                "welcome"
            }
            fn register(&self, bot: &mut Bot) -> std::result::Result<(), String> {
                bot.command("greet", "Use the revised greeting", |_context| async {
                    Ok(())
                })
                .map_err(|error| error.to_string())?;
                Ok(())
            }
        }

        let mut bot = BotBuilder::new().token("yb_test-token").build().unwrap();
        bot.add_cog(GoodCog).unwrap();
        bot.replace_cog(RevisedCog).unwrap();
        assert_eq!(bot.cogs(), vec!["welcome"]);
        let names = bot
            .command_definitions()
            .into_iter()
            .map(|definition| definition.name().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, ["greet"]);
    }

    #[test]
    fn rolls_back_all_registrations_when_cog_setup_fails() {
        let mut bot = BotBuilder::new().token("yb_test-token").build().unwrap();
        bot.command("duplicate", "Already registered", |_context| async {
            Ok(())
        })
        .unwrap();
        let result = bot.add_cog(FailingCog);
        assert!(
            matches!(result, Err(CogRegistrationError::SetupFailed { name, .. }) if name == "broken")
        );
        let names = bot.command_definitions();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].name(), "duplicate");
        assert!(bot.cogs().is_empty());
    }
}
