use super::*;

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn line(name: &str, hint: &str) -> String {
    let (name, hint) = (t(name), t(hint));
    I18n::english().t_with(
        "workspace.switcher_hint_line",
        &[("name", &name), ("hint", &hint)],
    )
}

#[test]
fn the_hint_names_each_entry_through_the_translation() {
    let text = hints(&I18n::english(), true);
    let expected = [
        line("workspace.browse", "workspace.browse_hint"),
        line("workspace.compare", "workspace.compare_hint"),
        line("workspace.shell", "workspace.shell_hint"),
    ]
    .join("\n");
    assert_eq!(text, expected);
}

#[test]
fn while_not_connected_the_hint_says_why_the_shell_is_off() {
    let text = hints(&I18n::english(), false);
    let shell = line("workspace.shell", "workspace.shell_hint_not_connected");
    assert!(text.ends_with(&shell), "{text}");
}
