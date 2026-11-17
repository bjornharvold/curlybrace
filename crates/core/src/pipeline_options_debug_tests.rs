//! `Debug` of the options names their shape, never a value: a comment, a hint or a locale is the
//! person's data and may end up in a log line through any type that holds the options.
use super::*;
use bson::doc;

const SECRET: &str = "s3cr3t-value";

fn full() -> PipelineOptions {
    PipelineOptions {
        collation: Some(CollationOption {
            locale: format!("{SECRET}_x"),
            strength: Some(Strength::Secondary),
            case_level: Some(true),
            case_first: None,
            numeric_ordering: None,
            alternate: None,
        }),
        hint: Some(HintOption::Keys(doc! { SECRET: 1 })),
        allow_disk_use: Some(true),
        max_time_ms: Some(4242),
        comment: Some(SECRET.into()),
        read_preference: Some(ReadPreferenceMode::Nearest),
    }
}

#[test]
fn the_debug_of_the_options_shows_no_value() {
    let shown = format!("{:?}", full());
    assert!(!shown.contains(SECRET), "{shown}");
    assert!(!shown.contains("4242"), "{shown}");
    let by_name = PipelineOptions {
        hint: Some(HintOption::Name(SECRET.into())),
        ..PipelineOptions::default()
    };
    assert!(!format!("{by_name:?}").contains(SECRET));
    assert!(
        !format!(
            "{:?}",
            CollationOption {
                locale: SECRET.into(),
                strength: None,
                case_level: None,
                case_first: None,
                numeric_ordering: None,
                alternate: None,
            }
        )
        .contains(SECRET)
    );
}

#[test]
fn the_debug_still_says_which_options_are_set() {
    let none = format!("{:?}", PipelineOptions::default());
    let some = format!("{:?}", full());
    assert_ne!(none, some);
    assert!(some.contains("comment"), "{some}");
    assert!(some.contains("hint"), "{some}");
}
