use std::convert::TryFrom;

use rusqlite::{params, Connection, OptionalExtension, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Role {
    Assistant,
    User,
}

impl TryFrom<String> for Role {
    type Error = String;

    fn try_from(role: String) -> Result<Self, Self::Error> {
        match role.to_lowercase().as_str() {
            "assistant" => Ok(Role::Assistant),
            "user" => Ok(Role::User),
            _ => Err(format!("Invalid role string: {}", role)),
        }
    }
}

impl Role {
    fn to_string(&self) -> String {
        match self {
            Role::Assistant => "Assistant".to_string(),
            Role::User => "User".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub message_id: i64,
    pub content: String,
    pub role: Role,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        let conn = if cfg!(test) {
            Connection::open_in_memory()?
        } else {
            Connection::open(path)?
        };

        let sql = r#"
-- Create messages table
CREATE TABLE IF NOT EXISTS messages (
    message_id INTEGER PRIMARY KEY AUTOINCREMENT,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Create FTS virtual table for messages
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    content,  -- the text field we want to search
    role,     -- optional: include if you want to search by role too
    content='messages',  -- this tells FTS which table to shadow
    content_rowid='message_id'  -- this specifies the primary key to link with
);

-- Create triggers to keep FTS table in sync with messages table
CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, content, role)
    VALUES (new.message_id, new.content, new.role);
END;

CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content, role)
    VALUES('delete', old.message_id, old.content, old.role);
END;

CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content, role)
    VALUES('delete', old.message_id, old.content, old.role);
    INSERT INTO messages_fts(rowid, content, role)
    VALUES (new.message_id, new.content, new.role);
END;

-- Create conversation table
CREATE TABLE IF NOT EXISTS conversation (
    conversation_id INTEGER PRIMARY KEY AUTOINCREMENT,
    title VARCHAR(255),
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Create thread table to link messages
CREATE TABLE IF NOT EXISTS thread (
    thread_id INTEGER PRIMARY KEY AUTOINCREMENT,
    previous_message_id INTEGER REFERENCES messages(message_id) NOT NULL,
    next_message_id INTEGER REFERENCES messages(message_id) NOT NULL,
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

    /// Creates a standalone message in the DB
    /// No associated thread here, should really only be used for one-offs
    /// and messages that aren't part of any sort of conversation
    pub fn create_message(&self, content: &str, role: Role) -> Result<i64> {
        let sql = "INSERT INTO messages (content, role) VALUES (?1, ?2)";
        self.conn.execute(sql, params![content, role.to_string()])?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Create a message in the DB with an associated thread
    /// This should be used to keep associated messages explicitly linked
    /// e.g., a response to a previously sent message
    pub fn create_message_with_thread(
        &mut self,
        content: &str,
        role: Role,
        previous_message_id: i64,
        conversation_id: i64,
    ) -> Result<(i64, i64)> {
        let tx = self.conn.transaction()?;

        // First, create the message
        let message_sql = "INSERT INTO messages (content, role) VALUES (?1, ?2)";
        tx.execute(message_sql, params![content, role.to_string()])?;
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
        let sql =
            "SELECT message_id, content, role, created_at FROM messages WHERE message_id = ?1";
        let mut stmt = self.conn.prepare(sql)?;

        let message = stmt
            .query_row(params![message_id], |row| {
                Ok(Message {
                    message_id: row.get(0)?,
                    content: row.get(1)?,
                    role: Role::try_from(row.get::<_, String>(2)?).unwrap(),
                    created_at: row.get(3)?,
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

    pub fn get_conversation(&self, title: &str) -> Result<Option<Conversation>> {
        let sql = "SELECT conversation_id, title, created_at FROM conversation WHERE title = ?1";
        let mut stmt = self.conn.prepare(sql)?;

        let conversation = stmt
            .query_row(params![title], |row| {
                Ok(Conversation {
                    conversation_id: row.get(0)?,
                    title: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .optional()?;

        Ok(conversation)
    }

    pub fn get_conversations(&self) -> Result<Vec<Conversation>> {
        let sql = "SELECT conversation_id, title, created_at FROM conversation";
        let mut stmt = self.conn.prepare(sql)?;

        let conversations = stmt
            .query_map(params![], |row| {
                Ok(Conversation {
                    conversation_id: row.get(0)?,
                    title: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<Conversation>, _>>()?;

        Ok(conversations)
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
            WHERE message_id = ?1 OR message_id = ?2";

        let message_count: i64 = self.conn.query_row(
            verify_messages_sql,
            params![previous_message_id, next_message_id],
            |row| row.get(0),
        )?;

        if message_count != 2 {
            panic!("A thread can only exist between 2 messages");
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

    pub fn get_last_updated_conversation(&self) -> Result<Option<Conversation>> {
        let sql = r#"
        SELECT DISTINCT c.conversation_id, c.title, c.created_at
        FROM conversation c
        JOIN thread t ON c.conversation_id = t.conversation_id
        JOIN messages m ON m.message_id = t.next_message_id
        ORDER BY m.created_at DESC
        LIMIT 1
    "#;

        let mut stmt = self.conn.prepare(sql)?;

        let conversation = stmt
            .query_row(params![], |row| {
                Ok(Conversation {
                    conversation_id: row.get(0)?,
                    title: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .optional()?;

        Ok(conversation)
    }

    // Get all messages in a conversation
    pub fn get_conversation_messages(&self, conversation_title: &str) -> Result<Vec<Message>> {
        let sql = "
            SELECT DISTINCT m.message_id, m.content, m.role, m.created_at FROM messages m
            JOIN conversation c ON c.title = ?1
            JOIN thread t ON (m.message_id = t.previous_message_id OR m.message_id = t.next_message_id) and t.conversation_id = c.conversation_id
            ORDER BY m.created_at
        ";

        let mut stmt = self.conn.prepare(sql)?;
        let messages = stmt
            .query_map(params![conversation_title], |row| {
                Ok(Message {
                    message_id: row.get(0)?,
                    content: row.get(1)?,
                    role: Role::try_from(row.get::<_, String>(2)?).unwrap(),
                    created_at: row.get(3)?,
                })
            })?
            .into_iter()
            .map(|m| m.unwrap())
            .collect::<Vec<Message>>();

        Ok(messages)
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
    fn test_db_init() {
        let db = test_setup();

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
    fn test_create_conversation() {
        let db = test_setup().unwrap();

        let id = db.create_conversation("testing!");
        assert!(id.is_ok());

        let id = id.unwrap();
        let conv = db.get_conversation(id);
        assert!(conv.is_ok());

        let conv = conv.unwrap();
        assert!(conv.is_some());

        let conv = conv.unwrap();
        assert_eq!(conv.title, "testing!");
        assert_eq!(conv.conversation_id, 1);
    }

    #[test]
    fn test_create_message_with_thread() {
        let mut db = test_setup().unwrap();

        // First, create a conversation.
        db.conn
            .execute("INSERT INTO conversation (title) VALUES ('Test Conv')", [])
            .unwrap();
        let conv_id = db.conn.last_insert_rowid();

        // Insert messages with threads.
        // Message 1: No previous message.
        let msg1 = db
            .create_message("First message", Role::try_from("User".to_string()).unwrap())
            .unwrap();
        // Message 2: Linked to msg1.
        let (msg2, thread_id) = db
            .create_message_with_thread(
                "Second message",
                Role::try_from("Assistant".to_string()).unwrap(),
                msg1,
                conv_id,
            )
            .unwrap();

        // Verify message exists.
        let msg = db.get_message(msg1).unwrap();
        assert!(msg.is_some());
        assert_eq!(msg.unwrap().content, "First message");

        // Verify thread exists.
        let thread = db.get_thread(thread_id).unwrap();
        assert!(thread.is_some());
        let thread = thread.unwrap();

        // previous_message_id is NULL, so it should be None (or 0 if treated as NULL)
        assert_eq!(thread.next_message_id, msg2);
    }

    #[test]
    fn test_create_thread() {
        let db = test_setup().unwrap();

        // Create conversation.
        db.conn
            .execute(
                "INSERT INTO conversation (title) VALUES ('Thread Test')",
                [],
            )
            .unwrap();
        let conv_id = db.conn.last_insert_rowid();

        // Manually insert two messages into the conversation.
        db.conn
            .execute(
                "INSERT INTO messages (content, role) VALUES (?1, ?2)",
                params!["Message A", "User"],
            )
            .unwrap();
        let msg_a = db.conn.last_insert_rowid();

        db.conn
            .execute(
                "INSERT INTO messages (content, role) VALUES (?1, ?2)",
                params!["Message B", "Assistant"],
            )
            .unwrap();
        let msg_b = db.conn.last_insert_rowid();

        // Create a thread linking these messages.
        let thread_id = db
            .create_thread(msg_a, msg_b, conv_id)
            .expect("Failed to create thread");

        // Verify the thread.
        let thread = db.get_thread(thread_id).unwrap().expect("Thread not found");
        assert_eq!(thread.previous_message_id, msg_a);
        assert_eq!(thread.next_message_id, msg_b);
        assert_eq!(thread.conversation_id, conv_id);
    }

    #[test]
    fn test_get_conversation_messages() {
        let mut db = test_setup().unwrap();

        // Create conversation.
        db.conn
            .execute(
                "INSERT INTO conversation (title) VALUES ('Message List')",
                [],
            )
            .unwrap();
        let conv_id = db.conn.last_insert_rowid();

        // Insert messages with threads.
        // Message 1: No previous message.
        let msg1 = db
            .create_message("First message", Role::try_from("User".to_string()).unwrap())
            .unwrap();
        // Message 2: Linked to msg1.
        let (msg2, _thread2) = db
            .create_message_with_thread(
                "Second message",
                Role::try_from("Assistant".to_string()).unwrap(),
                msg1,
                conv_id,
            )
            .unwrap();

        // Retrieve conversation messages.
        let messages = db.get_conversation_messages(conv_id).unwrap();
        // Both messages should be present.
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].message_id, msg1);
        assert_eq!(messages[1].message_id, msg2);
    }
}
