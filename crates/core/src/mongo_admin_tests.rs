//! The commands and reply parsing of database and collection administration. Pure: no server.

use bson::doc;

use crate::admin::{CollectionStats, DatabaseStats};
use crate::error::CoreError;
use crate::mongo_admin::{
    collection_stats_from, database_stats_from, drop_collection_command, map_admin_code,
    rename_command, view_command,
};
use crate::store::Namespace;

#[test]
fn the_rename_command_names_both_namespaces_and_the_drop_choice() {
    let ns = Namespace::new("shop", "old");
    assert_eq!(
        rename_command(&ns, "new", true),
        doc! { "renameCollection": "shop.old", "to": "shop.new", "dropTarget": true }
    );
    assert_eq!(
        rename_command(&ns, "new", false),
        doc! { "renameCollection": "shop.old", "to": "shop.new", "dropTarget": false }
    );
}

#[test]
fn the_view_command_carries_the_source_and_the_pipeline() {
    let pipeline = vec![doc! { "$match": { "a": 1 } }];
    assert_eq!(
        view_command("v", "src", &pipeline),
        doc! { "create": "v", "viewOn": "src", "pipeline": [ { "$match": { "a": 1 } } ] }
    );
}

#[test]
fn the_drop_command_names_the_collection() {
    assert_eq!(drop_collection_command("c"), doc! { "drop": "c" });
}

#[test]
fn collection_stats_read_counts_sizes_and_index_sizes() {
    let reply = doc! { "count": 12_i32, "size": 3000_i64, "storageSize": 4096.0,
    "totalIndexSize": 8192_i32, "indexSizes": { "_id_": 4096_i32, "a_1": 4096_i64 }, "ok": 1.0 };
    assert_eq!(
        collection_stats_from(&reply),
        CollectionStats {
            document_count: 12,
            data_size: 3000,
            storage_size: 4096,
            total_index_size: 8192,
            index_sizes: vec![("_id_".into(), 4096), ("a_1".into(), 4096)],
        }
    );
}

#[test]
fn collection_stats_of_a_sparse_reply_default_to_zero() {
    assert_eq!(
        collection_stats_from(&doc! { "ok": 1.0 }),
        CollectionStats::default()
    );
}

#[test]
fn database_stats_read_counts_and_sizes() {
    let reply = doc! { "collections": 3_i32, "views": 1_i32, "objects": 99_i64, "dataSize": 1000.0,
    "storageSize": 2000_i64, "indexSize": 500_i32, "ok": 1.0 };
    assert_eq!(
        database_stats_from(&reply),
        DatabaseStats {
            collections: 3,
            views: 1,
            document_count: 99,
            data_size: 1000,
            storage_size: 2000,
            total_index_size: 500,
        }
    );
}

#[test]
fn server_codes_map_to_typed_errors() {
    assert_eq!(map_admin_code(Some(26)), Some(CoreError::NotFound));
    assert_eq!(map_admin_code(Some(48)), Some(CoreError::NamespaceExists));
    assert_eq!(map_admin_code(Some(11)), None);
    assert_eq!(map_admin_code(None), None);
}

#[test]
fn the_view_code_is_not_a_general_admin_meaning() {
    // Only the statistics read gives 166 the "no statistics for a view" meaning.
    assert_eq!(map_admin_code(Some(166)), None);
}
