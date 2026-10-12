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
