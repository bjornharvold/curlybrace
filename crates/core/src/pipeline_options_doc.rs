//! The options of an aggregate as BSON, for code that spells them in a driver's own syntax.

use bson::{Bson, Document, doc};

use crate::pipeline_options::{Alternate, CaseFirst, CollationOption, HintOption};

impl CollationOption {
    /// The collation as MongoDB spells it: only the fields that are set, `strength` as an int32.
    pub fn to_document(&self) -> Document {
        let mut out = doc! { "locale": self.locale.as_str() };
        if let Some(strength) = self.strength {
            out.insert("strength", i32::from(u8::from(strength)));
        }
        if let Some(case_level) = self.case_level {
            out.insert("caseLevel", case_level);
        }
        if let Some(case_first) = self.case_first {
            let word = match case_first {
                CaseFirst::Upper => "upper",
                CaseFirst::Lower => "lower",
                CaseFirst::Off => "off",
            };
            out.insert("caseFirst", word);
        }
        if let Some(numeric_ordering) = self.numeric_ordering {
            out.insert("numericOrdering", numeric_ordering);
        }
        if let Some(alternate) = self.alternate {
            let word = match alternate {
                Alternate::NonIgnorable => "non-ignorable",
                Alternate::Shifted => "shifted",
            };
            out.insert("alternate", word);
        }
        out
    }
}

impl HintOption {
    /// The hint as the server takes it: the index name, or the key pattern document.
    pub fn to_bson(&self) -> Bson {
        match self {
            Self::Name(name) => Bson::String(name.clone()),
            Self::Keys(keys) => Bson::Document(keys.clone()),
        }
    }
}

#[cfg(test)]
#[path = "pipeline_options_doc_tests.rs"]
mod tests;
