//! `/account` in Desktop: open the Accounts panel, and switch between the
//! OAuth logins the runtime already holds.
//!
//! Desktop used to answer every `/account` with "not available in Desktop
//! yet", so a user with two Claude logins had no way to change which one the
//! runtime spends. The grammar mirrors the TUI's so muscle memory carries
//! over, including its error text for an ambiguous bare label.

use super::*;
use crate::accounts::Account;

/// What a parsed `/account` line asks for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AccountRequest {
    /// Bare `/account`: show the accounts surface, like `/login` does.
    Open,
    /// `/account switch <label>`, with the provider resolved from the
    /// snapshot when the user did not name one.
    Switch {
        provider: Option<String>,
        label: String,
    },
}

/// Parse an `/account` or `/accounts` line.
///
/// `None` means "not an account command", so unrelated input keeps its current
/// handling. `Some(Err(_))` is a usage message for a malformed account command.
pub(crate) fn parse_account_command(input: &str) -> Option<Result<AccountRequest, String>> {
    let trimmed = input.trim();
    let mut words = trimmed.split_whitespace();
    let name = words.next()?;
    if !matches!(name, "/account" | "/accounts") {
        return None;
    }
    let Some(first) = words.next() else {
        return Some(Ok(AccountRequest::Open));
    };
    // `/account <provider> switch <label>` names the credential explicitly.
    let (provider, verb) = match switch_provider_id(first) {
        Some(provider) => (Some(provider.to_owned()), words.next().unwrap_or_default()),
        None => (None, first),
    };
    if verb != "switch" {
        // Every other subcommand still belongs to the CLI, so let the caller
        // report it rather than pretending Desktop implements it.
        return None;
    }
    // Labels never contain spaces in the account store, but rejoining keeps a
    // pasted label with stray spacing from silently becoming a different one.
    let label = words.collect::<Vec<_>>().join(" ");
    if label.is_empty() {
        return Some(Err(match &provider {
            Some(provider) => format!("Usage: `/account {provider} switch <label>`."),
            None => "Usage: `/account switch <label>`.".into(),
        }));
    }
    Some(Ok(AccountRequest::Switch { provider, label }))
}

/// The runtime's switchable credential vocabulary, accepting the spellings a
/// user is likely to type for each one.
pub(crate) fn switch_provider_id(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "claude" | "anthropic" => Some("claude"),
        "openai" | "chatgpt" => Some("openai"),
        _ => None,
    }
}

/// Human name for a switchable provider, matching the CLI's phrasing.
pub(crate) fn switch_provider_label(provider: &str) -> &'static str {
    match provider {
        "openai" => "OpenAI",
        _ => "Anthropic",
    }
}

/// Which provider owns a bare `<label>`, decided from the latest snapshot.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LabelResolution {
    Provider(String),
    /// The same label exists on more than one provider.
    Ambiguous,
    Unknown,
}

/// Resolve a bare label against the accounts the runtime reported. Two
/// providers can legitimately use the same label, and guessing one would spend
/// the wrong subscription, so that case is reported rather than resolved.
pub(crate) fn resolve_label(accounts: &[Account], label: &str) -> LabelResolution {
    let mut found: Vec<String> = Vec::new();
    for account in accounts {
        let Some(provider) = account.switch_provider() else {
            continue;
        };
        let owns = account
            .usage_reports
            .iter()
            .any(|report| report.login_label().as_deref() == Some(label));
        if owns && !found.iter().any(|existing| existing == provider) {
            found.push(provider.to_owned());
        }
    }
    match found.len() {
        0 => LabelResolution::Unknown,
        1 => LabelResolution::Provider(found.remove(0)),
        _ => LabelResolution::Ambiguous,
    }
}

/// The TUI's message for a label both providers own. Keep it identical so the
/// documented recovery commands stay correct in both surfaces.
pub(crate) fn ambiguous_label_message(label: &str) -> String {
    format!(
        "Account label `{label}` exists for both Anthropic and OpenAI. \
         Use /account claude switch {label} or /account openai switch {label} explicitly."
    )
}

pub(crate) fn unknown_label_message(label: &str) -> String {
    format!("No Anthropic or OpenAI account with label `{label}` found.")
}

pub(crate) fn switched_message(provider: &str, label: &str) -> String {
    format!(
        "Switched to {} account `{label}`.",
        switch_provider_label(provider)
    )
}

impl Panel {
    /// Handle `/account` and `/accounts`. Returns whether the line was one.
    ///
    /// Bare `/account` is dispatched to the Accounts surface by the composer
    /// (like `/login`), so this path only has to answer `switch`.
    pub(super) fn account_command(&mut self, content: &str, cx: &mut Context<Self>) -> bool {
        let Some(parsed) = parse_account_command(content) else {
            return false;
        };
        match parsed {
            Err(usage) => self.items.push(Item::Error(usage)),
            // Reaching here means the composer could not dispatch the action
            // (no window, or a preview panel), so say where accounts live.
            Ok(AccountRequest::Open) => self.items.push(Item::Assistant(
                "Open the Accounts tab in the sidebar to sign in or switch logins, \
                 or use `/account switch <label>`."
                    .into(),
            )),
            Ok(AccountRequest::Switch { provider, label }) => {
                self.switch_account(provider, label, cx)
            }
        }
        true
    }

    /// Ask the runtime to spend a different stored login for a provider.
    pub(crate) fn switch_account(
        &mut self,
        provider: Option<String>,
        label: String,
        cx: &mut Context<Self>,
    ) {
        let provider = match provider {
            Some(provider) => provider,
            None => {
                let accounts = cx
                    .try_global::<crate::panel::usage::StatusAccounts>()
                    .map(|snapshot| snapshot.0.clone())
                    .unwrap_or_default();
                match resolve_label(&accounts, &label) {
                    LabelResolution::Provider(provider) => provider,
                    LabelResolution::Ambiguous => {
                        self.items.push(Item::Error(ambiguous_label_message(&label)));
                        return;
                    }
                    LabelResolution::Unknown => {
                        self.items.push(Item::Error(unknown_label_message(&label)));
                        return;
                    }
                }
            }
        };
        self.bridge.send(Command::SwitchAccount {
            provider: provider.clone(),
            label: label.clone(),
            session_id: self.session_id.clone(),
        });
        // The runtime confirms with `Update::AccountSwitched`; this line keeps
        // the transcript honest about what was requested meanwhile.
        self.items
            .push(Item::Assistant(switching_message(&provider, &label)));
    }
}

pub(crate) fn switching_message(provider: &str, label: &str) -> String {
    format!(
        "Switching to {} account `{label}`…",
        switch_provider_label(provider)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{UsageReport, parse};

    fn claude_and_openai(shared: bool) -> Vec<Account> {
        let mut accounts = parse(
            r#"{"providers":[
            {"id":"claude","display_name":"Anthropic/Claude","status":"available","auth_kind":"OAuth"},
            {"id":"openai","display_name":"OpenAI","status":"available","auth_kind":"OAuth"},
            {"id":"openrouter","display_name":"OpenRouter","status":"available","auth_kind":"API key"}
        ]}"#,
        )
        .unwrap();
        let report = |name: &str| UsageReport {
            provider_name: name.into(),
            account_label: None,
            limits: Vec::new(),
            extra_info: Vec::new(),
        };
        for account in &mut accounts {
            match account.id.as_str() {
                "claude" => {
                    account.usage_reports = vec![
                        report("Anthropic - claude-fox (s***k@gmail.com) ✦"),
                        report("Anthropic - claude-otter (a***h@gmail.com)"),
                    ];
                    if shared {
                        account
                            .usage_reports
                            .push(report("Anthropic - shared (x***x@gmail.com)"));
                    }
                }
                "openai" => {
                    account.usage_reports = vec![report("OpenAI - openai-otter (a***h@proton.me)")];
                    if shared {
                        account
                            .usage_reports
                            .push(report("OpenAI - shared (y***y@proton.me)"));
                    }
                }
                _ => {}
            }
        }
        accounts
    }

    #[test]
    fn bare_account_opens_the_panel_under_either_spelling() {
        for input in ["/account", "/accounts", "  /account  "] {
            assert_eq!(
                parse_account_command(input),
                Some(Ok(AccountRequest::Open)),
                "{input} should open the accounts surface"
            );
        }
    }

    #[test]
    fn switch_accepts_shorthand_and_explicit_provider_forms() {
        assert_eq!(
            parse_account_command("/account switch claude-otter"),
            Some(Ok(AccountRequest::Switch {
                provider: None,
                label: "claude-otter".into()
            }))
        );
        for (input, provider) in [
            ("/account claude switch claude-otter", "claude"),
            ("/account anthropic switch claude-otter", "claude"),
            ("/accounts openai switch claude-otter", "openai"),
            ("/account chatgpt switch claude-otter", "openai"),
        ] {
            assert_eq!(
                parse_account_command(input),
                Some(Ok(AccountRequest::Switch {
                    provider: Some(provider.into()),
                    label: "claude-otter".into()
                })),
                "{input}"
            );
        }
    }

    #[test]
    fn switch_without_a_label_explains_itself_per_form() {
        assert_eq!(
            parse_account_command("/account switch"),
            Some(Err("Usage: `/account switch <label>`.".into()))
        );
        assert_eq!(
            parse_account_command("/account openai switch"),
            Some(Err("Usage: `/account openai switch <label>`.".into()))
        );
    }

    #[test]
    fn other_commands_and_subcommands_are_left_alone() {
        assert_eq!(parse_account_command("/login openai"), None);
        assert_eq!(parse_account_command("hello"), None);
        assert_eq!(parse_account_command(""), None);
        // Still CLI-only, so the caller keeps reporting them honestly.
        assert_eq!(parse_account_command("/account doctor"), None);
        assert_eq!(parse_account_command("/account claude remove fox"), None);
    }

    #[test]
    fn labels_resolve_to_one_provider_or_report_the_ambiguity() {
        let accounts = claude_and_openai(false);
        assert_eq!(
            resolve_label(&accounts, "claude-otter"),
            LabelResolution::Provider("claude".into())
        );
        assert_eq!(
            resolve_label(&accounts, "openai-otter"),
            LabelResolution::Provider("openai".into())
        );
        assert_eq!(
            resolve_label(&accounts, "missing"),
            LabelResolution::Unknown
        );
        let shared = claude_and_openai(true);
        assert_eq!(resolve_label(&shared, "shared"), LabelResolution::Ambiguous);
    }

    #[test]
    fn messages_name_the_provider_and_the_explicit_recovery_commands() {
        let message = ambiguous_label_message("shared");
        assert!(message.contains("exists for both Anthropic and OpenAI"));
        assert!(message.contains("/account claude switch shared"));
        assert!(message.contains("/account openai switch shared"));
        assert!(unknown_label_message("nope").contains("`nope`"));
        assert_eq!(
            switched_message("claude", "claude-otter"),
            "Switched to Anthropic account `claude-otter`."
        );
        assert_eq!(
            switched_message("openai", "openai-otter"),
            "Switched to OpenAI account `openai-otter`."
        );
    }

    /// `/account switch` must reach the bridge, and an ambiguous bare label
    /// must stop rather than guess which subscription to spend.
    #[gpui::test]
    fn account_switch_command_sends_the_request_or_refuses_ambiguity(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            cx.set_global(crate::panel::usage::StatusAccounts(claude_and_openai(true)));
        });
        let (bridge, commands) = crate::harness::spawn_recording();
        let (workspace, vcx) = cx.add_window_view(|_, cx| {
            let mut workspace =
                crate::workspace::Workspace::for_test(crate::learning::Coach::new(), cx);
            workspace.set_test_bridge(bridge);
            workspace.push_test_panel("account-session", cx);
            workspace
        });
        let panel = workspace.read_with(vcx, |workspace, _| workspace.test_panel(0).unwrap());
        while commands.try_recv().is_ok() {}

        // Explicit provider: no snapshot lookup needed.
        panel.update(vcx, |panel, cx| {
            assert!(panel.handle_slash_command("/account openai switch openai-otter", cx));
        });
        assert!(
            matches!(
                commands.try_recv(),
                Ok(crate::harness::Command::SwitchAccount { provider, label, session_id })
                    if provider == "openai" && label == "openai-otter"
                        && session_id == "account-session"
            ),
            "an explicit provider switches without consulting the snapshot"
        );

        // Shorthand resolved from the snapshot.
        panel.update(vcx, |panel, cx| {
            assert!(panel.handle_slash_command("/account switch claude-otter", cx));
        });
        assert!(matches!(
            commands.try_recv(),
            Ok(crate::harness::Command::SwitchAccount { provider, label, .. })
                if provider == "claude" && label == "claude-otter"
        ));

        // A label both providers own is reported, never guessed.
        panel.update(vcx, |panel, cx| {
            panel.items.clear();
            assert!(panel.handle_slash_command("/account switch shared", cx));
            let last = panel.items.last().expect("an error is shown");
            match last {
                crate::panel::Item::Error(message) => {
                    assert!(message.contains("exists for both Anthropic and OpenAI"));
                    assert!(message.contains("/account claude switch shared"));
                }
                other => panic!("expected an ambiguity error, got {other:?}"),
            }
        });
        assert!(
            commands.try_recv().is_err(),
            "an ambiguous label must not spend either subscription"
        );

        // An unknown label is equally explicit.
        panel.update(vcx, |panel, cx| {
            panel.items.clear();
            assert!(panel.handle_slash_command("/accounts switch nope", cx));
            assert!(matches!(
                panel.items.last(),
                Some(crate::panel::Item::Error(message)) if message.contains("`nope`")
            ));
        });
        assert!(commands.try_recv().is_err());
    }
}
