use bson::{Bson, doc};

use super::*;

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn aggregate(stages: Vec<Document>) -> CodeRequest {
    CodeRequest::Aggregate { ns: ns(), stages }
}

fn gen_agg(language: Language, stages: Vec<Document>, options: &CodeOptions) -> String {
    generate(language, &aggregate(stages), options).expect("generates")
}

fn first_value(language: Language, value: Bson) -> Result<String, CodegenError> {
    let request = aggregate(vec![doc! { "$match": { "v": value } }]);
    generate(language, &request, &CodeOptions::default())
}

const FIRST_THREE: [Language; 3] = [Language::Mongosh, Language::Node, Language::Python];

#[test]
fn default_options_are_valid() {
    assert!(CodeOptions::default().validated().is_ok());
}

#[test]
fn unsafe_variable_names_are_refused() {
    let long = "a".repeat(42);
    for bad in [
        "", "1a", "a-b", "a b", "a;b", "a\"b", "a.b", "é", "_", "class", "await", "None", "self",
        "PUBLIC", "pipeline", "use", &long,
    ] {
        let options = CodeOptions {
            collection_var: bad.into(),
            ..CodeOptions::default()
        };
        assert_eq!(
            options.validated(),
            Err(CodegenError::InvalidName("collection")),
            "{bad:?}"
        );
    }
}

#[test]
fn each_option_reports_which_name_is_bad() {
    let client = CodeOptions {
        client_var: "for".into(),
        ..CodeOptions::default()
    };
    assert_eq!(client.validated(), Err(CodegenError::InvalidName("client")));
    let db = CodeOptions {
        db_var: "x y".into(),
        ..CodeOptions::default()
    };
    assert_eq!(db.validated(), Err(CodegenError::InvalidName("db")));
}

#[test]
fn names_must_be_distinct_and_must_not_shadow_the_shell_db() {
    let same = CodeOptions {
        collection_var: "client".into(),
        ..CodeOptions::default()
    };
    assert!(same.validated().is_err());
    let shadow = CodeOptions {
        db_var: "mydb".into(),
        collection_var: "db".into(),
        ..CodeOptions::default()
    };
    assert!(shadow.validated().is_err());
    let custom = CodeOptions {
        client_var: "c1".into(),
        db_var: "_database".into(),
        collection_var: "orders_2".into(),
        ..CodeOptions::default()
    };
    assert!(custom.validated().is_ok());
}

#[test]
fn custom_names_appear_in_the_output_and_bad_ones_never_do() {
    let options = CodeOptions {
        client_var: "mongo".into(),
        db_var: "shop_db".into(),
        collection_var: "orders_c".into(),
        ..CodeOptions::default()
    };
    let out = gen_agg(Language::Python, vec![doc! { "$match": {} }], &options);
    assert!(out.contains("mongo = MongoClient("));
    assert!(out.contains("shop_db = mongo[\"shop\"]"));
    assert!(out.contains("orders_c = shop_db[\"orders\"]"));
    let bad = CodeOptions {
        collection_var: "x\"); drop()".into(),
        ..CodeOptions::default()
    };
    let err = generate(Language::Node, &aggregate(vec![doc! {}]), &bad).unwrap_err();
    assert_eq!(err.error_variant(), "InvalidName");
    assert!(!err.to_string().contains("drop"));
}

#[test]
fn an_empty_pipeline_is_an_error_not_empty_code() {
    for language in FIRST_THREE {
        assert_eq!(
            generate(language, &aggregate(vec![]), &CodeOptions::default()),
            Err(CodegenError::EmptyPipeline)
        );
    }
}

#[test]
fn a_write_stage_is_exported_or_dropped_by_the_option() {
    let stages = vec![doc! { "$match": { "a": 1 } }, doc! { "$out": "totals" }];
    for language in FIRST_THREE {
        let on = gen_agg(language, stages.clone(), &CodeOptions::default());
        assert!(on.contains("$out"), "{language:?}");
        let off = CodeOptions {
            write_stage: false,
            ..CodeOptions::default()
        };
        let out = gen_agg(language, stages.clone(), &off);
        assert!(
            !out.contains("$out") && !out.contains("totals"),
            "{language:?}"
        );
        assert!(out.contains("$match"));
    }
}

#[test]
fn only_a_write_stage_with_the_option_off_is_an_empty_pipeline() {
    let off = CodeOptions {
        write_stage: false,
        ..CodeOptions::default()
    };
    let merge = vec![doc! { "$merge": { "into": "x" } }];
    assert_eq!(
        generate(Language::Node, &aggregate(merge), &off),
        Err(CodegenError::EmptyPipeline)
    );
}

#[test]
fn setup_off_gives_only_the_operation() {
    let options = CodeOptions {
        setup: false,
        ..CodeOptions::default()
    };
    for language in FIRST_THREE {
        let out = gen_agg(language, vec![doc! { "$match": { "a": 1 } }], &options);
        assert!(!out.contains("connection string"), "{language:?}");
        assert!(!out.contains("MongoClient"), "{language:?}");
        assert!(
            out.contains("collection.aggregate(pipeline)"),
            "{language:?}"
        );
    }
}

#[test]
fn the_snippet_connects_with_the_placeholder_only() {
    for language in FIRST_THREE {
        let out = gen_agg(
            language,
            vec![doc! { "$match": {} }],
            &CodeOptions::default(),
        );
        assert!(out.contains("<your connection string>"), "{language:?}");
        assert!(!out.contains("mongodb://") && !out.contains("mongodb+srv"));
        assert!(out.ends_with('\n'));
    }
}

#[test]
fn language_keys_are_stable_and_unique() {
    let keys: Vec<_> = Language::ALL.iter().map(|l| l.key()).collect();
    assert_eq!(
        keys,
        [
            "mongosh", "node", "python", "java", "csharp", "go", "php", "ruby", "rust"
        ]
    );
}

#[test]
fn text_cannot_break_out_of_a_string_literal() {
    let nasty = "\"); process.exit(1); //\\ ${x} #{y} \n\t\u{0}";
    for language in FIRST_THREE {
        let out = first_value(language, Bson::String(nasty.into())).unwrap();
        assert!(
            out.contains("\\\"); process.exit(1); //\\\\ ${x} #{y} \\n\\t"),
            "{language:?}"
        );
        assert!(!out.contains(" \"); process"), "{language:?}");
        assert_eq!(out.matches("process.exit").count(), 1, "{language:?}");
    }
}

#[test]
fn a_proto_key_does_not_set_the_prototype() {
    let out = first_value(Language::Node, Bson::Document(doc! { "__proto__": 1_i32 })).unwrap();
    assert!(out.contains("[\"__proto__\"]: 1"));
    let out = first_value(
        Language::Node,
        Bson::Document(doc! { "a.b": 1_i32, "$x": 2_i32 }),
    );
    let out = out.unwrap();
    assert!(out.contains("\"a.b\": 1") && out.contains("$x: 2"));
}

#[test]
fn doubles_stay_doubles_and_non_finite_values_are_spelled() {
    let node = first_value(Language::Node, Bson::Double(1.0)).unwrap();
    assert!(node.contains("new Double(1.0)"));
    assert!(
        first_value(Language::Node, Bson::Double(1.5))
            .unwrap()
            .contains("v: 1.5")
    );
    assert!(
        first_value(Language::Mongosh, Bson::Double(f64::NAN))
            .unwrap()
            .contains("v: NaN")
    );
    let inf = first_value(Language::Mongosh, Bson::Double(f64::NEG_INFINITY)).unwrap();
    assert!(inf.contains("-Infinity"));
    let py = first_value(Language::Python, Bson::Double(f64::INFINITY)).unwrap();
    assert!(py.contains("float(\"inf\")"));
    assert!(
        first_value(Language::Python, Bson::Double(1.0))
            .unwrap()
            .contains("\"v\": 1.0")
    );
}

#[test]
fn integer_widths_are_kept() {
    let py = first_value(Language::Python, Bson::Int64(5)).unwrap();
    assert!(py.contains("Int64(5)") && py.contains("from bson.int64 import Int64"));
    assert!(
        first_value(Language::Mongosh, Bson::Int32(5))
            .unwrap()
            .contains("v: 5 }")
    );
    assert!(
        first_value(Language::Mongosh, Bson::Int64(5))
            .unwrap()
            .contains("NumberLong(\"5\")")
    );
    let long = first_value(Language::Node, Bson::Int64(i64::MIN)).unwrap();
    assert!(long.contains("Long.fromString(\"-9223372036854775808\")"));
}

#[test]
fn dates_before_1970_and_out_of_range_are_expressible() {
    let before = Bson::DateTime(bson::DateTime::from_millis(-1));
    assert!(
        first_value(Language::Mongosh, before.clone())
            .unwrap()
            .contains("ISODate(\"1969-12-31T23:59:59.999Z\")")
    );
    assert!(first_value(Language::Python, before).unwrap().contains(
        "datetime.datetime(1969, 12, 31, 23, 59, 59, 999000, tzinfo=datetime.timezone.utc)"
    ));
    let far = Bson::DateTime(bson::DateTime::from_millis(253_402_300_800_000));
    assert!(
        first_value(Language::Node, far.clone())
            .unwrap()
            .contains("new Date(253402300800000)")
    );
    assert!(
        first_value(Language::Python, far)
            .unwrap()
            .contains("DatetimeMS(253402300800000)")
    );
    let beyond = Bson::DateTime(bson::DateTime::from_millis(i64::MAX));
    assert_eq!(
        first_value(Language::Node, beyond.clone()),
        Err(CodegenError::UnsupportedValue("dateTime"))
    );
    assert!(first_value(Language::Python, beyond).is_ok());
}

#[test]
fn a_uuid_binary_uses_the_uuid_helper_other_binaries_use_base64() {
    use bson::Binary;
    use bson::spec::BinarySubtype;
    let uuid = Bson::Binary(Binary {
        subtype: BinarySubtype::Uuid,
        bytes: (0..16).collect(),
    });
    let out = first_value(Language::Mongosh, uuid).unwrap();
    assert!(out.contains("UUID(\"00010203-0405-0607-0809-0a0b0c0d0e0f\")"));
    let odd = Bson::Binary(Binary {
        subtype: BinarySubtype::Uuid,
        bytes: vec![1, 2, 3],
    });
    assert!(
        first_value(Language::Mongosh, odd)
            .unwrap()
            .contains("BinData(4, \"AQID\")")
    );
    let user = Bson::Binary(Binary {
        subtype: BinarySubtype::UserDefined(0x80),
        bytes: vec![],
    });
    let node = first_value(Language::Node, user.clone()).unwrap();
    assert!(node.contains("new Binary(Buffer.from(\"\", \"base64\"), 128)"));
    assert!(
        first_value(Language::Python, user)
            .unwrap()
            .contains("Binary(bytes.fromhex(\"\"), 128)")
    );
}

#[test]
fn node_imports_only_the_classes_it_uses() {
    let out = gen_agg(
        Language::Node,
        vec![
            doc! { "$match": { "_id": bson::oid::ObjectId::parse_str("65a1b2c3d4e5f60718293a4b").unwrap(), "n": 1_i64 } },
        ],
        &CodeOptions::default(),
    );
    assert!(out.starts_with("const { Long, MongoClient, ObjectId } = require(\"mongodb\");"));
}

#[test]
fn a_db_pointer_becomes_a_dbref() {
    let pointer = Bson::try_from(serde_json::json!({
        "$dbPointer": { "$ref": "shop.orders", "$id": { "$oid": "65a1b2c3d4e5f60718293a4b" } }
    }))
    .expect("db pointer");
    assert!(matches!(pointer, Bson::DbPointer(_)));
    let out = first_value(Language::Python, pointer).unwrap();
    assert!(out.contains("DBRef(\"shop.orders\", ObjectId(\"65a1b2c3d4e5f60718293a4b\"))"));
}

#[test]
fn find_options_are_omitted_when_unset() {
    let request = CodeRequest::Find {
        ns: ns(),
        query: Query {
            filter: doc! { "a": 1_i32 },
            sort: None,
            projection: None,
            skip: 0,
            limit: 0,
        },
    };
    let options = CodeOptions {
        setup: false,
        ..CodeOptions::default()
    };
    let node = generate(Language::Node, &request, &options).unwrap();
    assert!(node.contains("await collection.find(filter).toArray();"));
    assert!(!node.contains("limit") && !node.contains("skip"));
}
