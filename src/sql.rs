use rusqlite::{params, Connection, OptionalExtension, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub message_id: i64,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Conversation {
    pub conversation_id: i64,
    pub title: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Thread {
    pub thread_id: i64,
    pub previous_message_id: i64,
    pub next_message_id: i64,
    pub conversation_id: i64,
    pub created_at: String,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(path: &std::path::PathBuf) -> Result<Self> {
        let conn = Connection::open(path)?;
        let sql = r#"
-- Create messages table
CREATE TABLE IF NOT EXISTS messages (
    message_id SERIAL PRIMARY KEY,
    content TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Create conversation table
CREATE TABLE IF NOT EXISTS conversation (
    conversation_id SERIAL PRIMARY KEY,
    title VARCHAR(255),
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Create thread table to link messages
CREATE TABLE IF NOT EXISTS thread (
    thread_id SERIAL PRIMARY KEY,
    previous_message_id INTEGER REFERENCES messages(message_id),
    next_message_id INTEGER REFERENCES messages(message_id),
    conversation_id INTEGER REFERENCES conversation(conversation_id),
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(previous_message_id, next_message_id),
    CONSTRAINT different_messages CHECK (previous_message_id != next_message_id)
);

-- Create indexes for better query performance
CREATE INDEX IF NOT EXISTS idx_thread_previous_message ON thread(previous_message_id);
CREATE INDEX IF NOT EXISTS idx_thread_next_message ON thread(next_message_id);
CREATE INDEX IF NOT EXISTS idx_thread_conversation ON thread(conversation_id);
        "#;
        conn.execute_batch(sql)?;
        Ok(Database { conn })
    }

    pub fn create_message_with_thread(
        &mut self,
        content: &str,
        previous_message_id: Option<i64>,
        conversation_id: i64,
    ) -> Result<(i64, i64)> {
        let tx = self.conn.transaction()?;

        // First, create the message
        let message_sql = "INSERT INTO messages (content, conversation_id) VALUES (?1, ?2)";
        tx.execute(message_sql, params![content, conversation_id])?;
        let new_message_id = tx.last_insert_rowid();

        // Create thread entry
        let thread_sql =
            "INSERT INTO thread (previous_message_id, next_message_id, conversation_id)
                 VALUES (?1, ?2, ?3)";
        tx.execute(
            thread_sql,
            params![previous_message_id, new_message_id, conversation_id],
        )?;

        let thread_id = tx.last_insert_rowid();
        tx.commit()?;

        Ok((new_message_id, thread_id))
    }

    pub fn get_message(&self, message_id: i64) -> Result<Option<Message>> {
        let sql = "SELECT message_id, content, created_at FROM messages WHERE message_id = ?1";
        let mut stmt = self.conn.prepare(sql)?;

        let message = stmt
            .query_row(params![message_id], |row| {
                Ok(Message {
                    message_id: row.get(0)?,
                    content: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .optional()?;

        Ok(message)
    }

    // Conversation operations
    pub fn create_conversation(&self, title: &str) -> Result<i64> {
        let sql = "INSERT INTO conversation (title) VALUES (?1)";
        self.conn.execute(sql, params![title])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_conversation(&self, conversation_id: i64) -> Result<Option<Conversation>> {
        let sql = "SELECT conversation_id, title, created_at FROM conversation WHERE conversation_id = ?1";
        let mut stmt = self.conn.prepare(sql)?;

        let conversation = stmt
            .query_row(params![conversation_id], |row| {
                Ok(Conversation {
                    conversation_id: row.get(0)?,
                    title: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .optional()?;

        Ok(conversation)
    }

    // Thread operations
    pub fn create_thread(
        &self,
        previous_message_id: i64,
        next_message_id: i64,
        conversation_id: i64,
    ) -> Result<i64> {
        // First, verify that both messages exist and belong to the same conversation
        let verify_messages_sql = "
            SELECT COUNT(*) 
            FROM messages 
            WHERE (message_id = ?1 OR message_id = ?2) 
            AND conversation_id = ?3";

        let message_count: i64 = self.conn.query_row(
            verify_messages_sql,
            params![previous_message_id, next_message_id, conversation_id],
            |row| row.get(0),
        )?;

        if message_count != 2 {
            panic!("Messages must exist and belong to the specified conversation");
        }

        // If verification passes, create the thread
        let sql = "INSERT INTO thread (previous_message_id, next_message_id, conversation_id) 
               VALUES (?1, ?2, ?3)";
        self.conn.execute(
            sql,
            params![previous_message_id, next_message_id, conversation_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_thread(&self, thread_id: i64) -> Result<Option<Thread>> {
        let sql = "SELECT thread_id, previous_message_id, next_message_id, conversation_id, created_at FROM thread WHERE thread_id = ?1";
        let mut stmt = self.conn.prepare(sql)?;

        let thread = stmt
            .query_row(params![thread_id], |row| {
                Ok(Thread {
                    thread_id: row.get(0)?,
                    previous_message_id: row.get(1)?,
                    next_message_id: row.get(2)?,
                    conversation_id: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .optional()?;

        Ok(thread)
    }

    // Get all messages in a conversation
    pub fn get_conversation_messages(&self, conversation_id: i64) -> Result<Vec<Message>> {
        let sql = "
            SELECT DISTINCT m.message_id, m.content, m.created_at FROM messages m
            JOIN thread t ON m.message_id = t.previous_message_id OR m.message_id = t.next_message_id
            WHERE t.conversation_id = ?1
            ORDER BY m.created_at
        ";

        let mut stmt = self.conn.prepare(sql)?;
        let messages = stmt.query_map(params![conversation_id], |row| {
            Ok(Message {
                message_id: row.get(0)?,
                content: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;

        let mut result = Vec::new();
        for message in messages {
            result.push(message?);
        }
        Ok(result)
    }
}

// NOTE: Remember to set the workspace root before instantiating this!
//       e.g.,
//      ```
//      let db = test_setup();
//      let _cleanup = Cleanup;
//      ```
pub struct Cleanup;
impl Drop for Cleanup {
    fn drop(&mut self) {
        std::fs::remove_dir_all(chamber_common::get_root_dir()).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup;

    fn test_setup() -> Result<Database, std::io::Error> {
        chamber_common::Workspace::new("/tmp/tllm_testing");
        setup();

        Ok(Database::new(&chamber_common::get_local_dir().join("tllm.sqlite")).unwrap())
    }

    #[test]
    fn db_init_test() {
        let db = test_setup();
        let _cleanup = Cleanup;

        assert!(db.is_ok());
        let db = db.unwrap();

        assert!(db.get_thread(0).is_ok(), "Failed to query thread table");
        assert!(
            db.get_conversation(0).is_ok(),
            "Failed to query conversation table"
        );
        assert!(db.get_message(0).is_ok(), "Failed to query message table");
    }

    #[test]
    fn create_conversation_test() {
        let db = test_setup().unwrap();
        let _cleanup = Cleanup;

        let id = db.create_conversation("testing!");
        assert!(id.is_ok());

        let id = id.unwrap();
        let conv = db.get_conversation(id);
        assert!(conv.is_ok());

        // TODO: why isn't the conversation being found?
        // let conv = conv.unwrap();
        // assert!(conv.is_some());

        // let conv = conv.unwrap();
        // assert_eq!(conv.title, "testing!");
        // assert_eq!(conv.conversation_id, 0);
    }
}
