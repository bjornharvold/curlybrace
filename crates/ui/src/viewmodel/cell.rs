//! Readable text for a single BSON value in a table cell.
use crate::I18n;
use bson::Bson;

/// Scalars as readable text; documents, arrays and binary as translated summaries.
pub fn cell_text(b: &Bson, i18n: &I18n) -> String {
    match b {
        Bson::String(s) => s.clone(),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => n.to_string(),
        Bson::Double(d) => d.to_string(),
        Bson::Boolean(v) => v.to_string(),
        Bson::ObjectId(oid) => oid.to_hex(),
        Bson::Decimal128(d) => d.to_string(),
        Bson::DateTime(dt) => dt
            .try_to_rfc3339_string()
            .unwrap_or_else(|_| dt.timestamp_millis().to_string()),
        Bson::Null => i18n.t("cell.null"),
        Bson::Undefined => i18n.t("cell.undefined"),
        Bson::MinKey => i18n.t("cell.min_key"),
        Bson::MaxKey => i18n.t("cell.max_key"),
        Bson::Document(d) => count_text(i18n, "cell.document", d.len()),
        Bson::Array(a) => count_text(i18n, "cell.array", a.len()),
        Bson::Binary(bin) => count_text(i18n, "cell.binary", bin.bytes.len()),
        Bson::RegularExpression(re) => format!("/{}/{}", re.pattern, re.options),
        Bson::Timestamp(ts) => i18n.t_with(
            "cell.timestamp",
            &[
                ("time", &ts.time.to_string()),
                ("increment", &ts.increment.to_string()),
            ],
        ),
        Bson::JavaScriptCode(code) => code.clone(),
        Bson::Symbol(s) => s.clone(),
        other => other.to_string(),
    }
}

fn count_text(i18n: &I18n, key: &str, n: usize) -> String {
    i18n.t_with(key, &[("n", &n.to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{Document, doc, oid::ObjectId};
    use mongostudio_core::test_support::mock_objects::exotic_doc;

    fn text(b: &Bson) -> String {
        cell_text(b, &I18n::english())
    }

    #[test]
    fn every_exotic_field_has_text_and_no_missing_key_marker() {
        for (k, v) in exotic_doc() {
            let t = text(&v);
            assert!(!t.is_empty(), "{k}");
            assert!(!t.contains('⟦'), "{k}: {t}");
        }
    }

    #[test]
    fn out_of_range_date_falls_back_to_millis() {
        let dt = bson::DateTime::from_millis(i64::MAX);
        assert_eq!(text(&Bson::DateTime(dt)), i64::MAX.to_string());
    }

    #[test]
    fn scalars() {
        let oid = ObjectId::new();
        assert_eq!(text(&Bson::ObjectId(oid)), oid.to_hex());
        assert_eq!(text(&Bson::String("héllo".into())), "héllo");
        assert_eq!(text(&Bson::Int32(7)), "7");
        assert_eq!(text(&Bson::Boolean(true)), "true");
        assert_eq!(text(&Bson::Double(f64::NAN)), "NaN");
        assert_eq!(
            text(&Bson::DateTime(bson::DateTime::from_millis(0))),
            "1970-01-01T00:00:00Z"
        );
    }

    #[test]
    fn composite_sizes_are_translated() {
        assert_eq!(text(&Bson::Document(doc! { "a": 1, "b": 2 })), "2 fields");
        assert_eq!(
            text(&Bson::Array(vec![1.into(), 2.into(), 3.into()])),
            "3 items"
        );
        assert_eq!(text(&Bson::Document(Document::new())), "0 fields");
    }

    #[test]
    fn binary_reports_byte_count() {
        let b = Bson::Binary(bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: vec![1, 2, 3, 4],
        });
        assert_eq!(text(&b), "Binary (4 bytes)");
    }

    #[test]
    fn null_and_keys_are_translated() {
        assert_eq!(text(&Bson::Null), "null");
        assert_eq!(text(&Bson::MinKey), "MinKey");
        assert_eq!(text(&Bson::MaxKey), "MaxKey");
    }
}
