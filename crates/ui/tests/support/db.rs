//! Direct driver access, used to seed data and to verify what the UI actually did.
use bson::{Document, doc};
use mongodb::Client;
use tokio::runtime::Handle;

/// Blocking helpers over the driver for one throwaway database.
pub struct TestDb {
    pub client: Client,
    pub name: String,
    rt: Handle,
}

fn block<F: IntoFuture>(rt: &Handle, f: F) -> F::Output {
    rt.block_on(f.into_future())
}

impl TestDb {
    pub fn new(client: Client, rt: Handle) -> Self {
        let name = format!("msit_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        Self { client, name, rt }
    }

    pub fn insert(&self, collection: &str, docs: Vec<Document>) {
        let coll = self
            .client
            .database(&self.name)
            .collection::<Document>(collection);
        block(&self.rt, coll.insert_many(docs)).expect("seed insert_many");
    }

    /// Every document of `collection`, ordered by `_id` for stable comparisons.
    pub fn all(&self, collection: &str) -> Vec<Document> {
        let coll = self
            .client
            .database(&self.name)
            .collection::<Document>(collection);
        block(&self.rt, async {
            use mongodb::options::FindOptions;
            let opts = FindOptions::builder().sort(doc! {"_id": 1}).build();
            let mut cursor = coll.find(doc! {}).with_options(opts).await?;
            let mut out = Vec::new();
            while cursor.advance().await? {
                out.push(cursor.deserialize_current()?);
            }
            Ok::<_, mongodb::error::Error>(out)
        })
        .expect("read back documents")
    }

    pub fn find_one(&self, collection: &str, filter: Document) -> Option<Document> {
        let coll = self
            .client
            .database(&self.name)
            .collection::<Document>(collection);
        block(&self.rt, coll.find_one(filter)).expect("find_one")
    }

    pub fn count(&self, collection: &str) -> u64 {
        let coll = self
            .client
            .database(&self.name)
            .collection::<Document>(collection);
        block(&self.rt, coll.count_documents(doc! {})).expect("count")
    }

    /// Best effort cleanup of this test's database.
    pub fn drop_database(&self) {
        let db = self.client.database(&self.name);
        if let Err(e) = block(&self.rt, db.drop()) {
            eprintln!("could not drop test database {}: {e}", self.name);
        }
    }
}

/// The three-person sample the landing page uses. No real personal data.
pub fn people() -> Vec<Document> {
    vec![
        doc! {"_id": 1_i32, "name": "Ada Lovelace", "plan": "pro", "seats": 12_i32, "active": true},
        doc! {"_id": 2_i32, "name": "Grace Hopper", "plan": "team", "seats": 40_i32, "active": true},
        doc! {"_id": 3_i32, "name": "Alan Turing", "plan": "free", "seats": 3_i32, "active": false},
    ]
}

/// The landing page's sample documents with richer BSON types, for the screenshots.
pub fn sample_people() -> Vec<Document> {
    use bson::{DateTime, Decimal128, oid::ObjectId};
    let row = |oid: &str,
               name: &str,
               plan: &str,
               seats: i32,
               credit: &str,
               joined: &str,
               active: bool| {
        doc! {
            "_id": ObjectId::parse_str(oid).expect("fixed object id"),
            "name": name,
            "plan": plan,
            "seats": seats,
            "credit": credit.parse::<Decimal128>().expect("fixed decimal"),
            "joinedAt": DateTime::parse_rfc3339_str(joined).expect("fixed date"),
            "active": active,
        }
    };
    vec![
        row(
            "66f1c0a2b7d4e8a1c3f50912",
            "Ada Lovelace",
            "pro",
            12,
            "1048.50",
            "2024-03-14T00:00:00Z",
            true,
        ),
        row(
            "66f1c0a2b7d4e8a1c3f50913",
            "Grace Hopper",
            "team",
            40,
            "2210.00",
            "2024-01-22T00:00:00Z",
            true,
        ),
        row(
            "66f1c0a2b7d4e8a1c3f50914",
            "Alan Turing",
            "free",
            3,
            "0.00",
            "2023-11-02T00:00:00Z",
            false,
        ),
    ]
}
