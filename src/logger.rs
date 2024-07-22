use std::io::Write;
use std::sync::Once;

pub struct Logger {
    filename: String,
}

static mut INSTANCE: Option<Logger> = None;
static INIT: Once = Once::new();

impl Logger {
    pub fn init(filename: String) -> &'static Logger {
        unsafe {
            INIT.call_once(|| {
                INSTANCE = Some(Logger { filename });
            });

            INSTANCE.as_ref().unwrap()
        }
    }

    #[allow(dead_code)]
    pub fn info(message: String) {
        unsafe {
            if INSTANCE.is_none() {
                panic!("Logger not initialized");
            }

            let filename = INSTANCE.as_ref().unwrap().filename.clone();
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(filename)
                .expect("Failed to open log file");

            let message = format!("{} [INFO]: {}\n", chrono::Local::now(), message);
            writeln!(file, "{}", message).expect("Failed to write to log file");
        }
    }

    pub fn error(message: String) {
        unsafe {
            if INSTANCE.is_none() {
                panic!("Logger not initialized");
            }

            let filename = INSTANCE.as_ref().unwrap().filename.clone();
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(filename)
                .expect("Failed to open log file");

            let message = format!("{} [ERROR]: {}\n", chrono::Local::now(), message);
            writeln!(file, "{}", message).expect("Failed to write to log file");
        }
    }
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        Logger::info(format!($($arg)*));
    }
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        Logger::error(format!($($arg)*));
    }
}
