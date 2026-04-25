use clap::{Parser, Subcommand};
use std::io::{self, Write};
use std::path::PathBuf;

use ForgeEngine::Db;

const DEFAULT_DB_ROOT: &str = "./forge_data";

#[derive(Debug, Parser)]
#[command(name = "forge", about = "Forge LSM storage CLI")]
struct Cli {
    #[arg(long, default_value = DEFAULT_DB_ROOT)]
    db_path: String,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Put { key: String, value: String },
    Get { key: String },
    Delete { key: String },
    Sync,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Some(command) => run_command(&cli.db_path, command),
        None => run_repl(&cli.db_path),
    }
}

fn run_repl(initial_db_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut current_db_path = initial_db_path.to_string();
    let mut db = Db::open(&current_db_path)?;
    println!("Forge REPL - type HELP for commands, EXIT to quit");

    let stdin = io::stdin();
    loop {
        print!("forge> ");
        io::stdout().flush()?;

        let mut line = String::new();
        let bytes = stdin.read_line(&mut line)?;
        if bytes == 0 {
            println!();
            break;
        }

        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }

        let cmd = tokens[0].to_ascii_uppercase();
        match cmd.as_str() {
            "PUT" => {
                if tokens.len() < 3 {
                    eprintln!("usage: PUT <key> <value>");
                    continue;
                }
                let key = tokens[1].to_string();
                let value = tokens[2..].join(" ");
                db.put(key, value.into_bytes())?;
                println!("ok");
            }
            "GET" => {
                if tokens.len() != 2 {
                    eprintln!("usage: GET <key>");
                    continue;
                }
                match db.get(tokens[1])? {
                    Some(value) => println!("{}", String::from_utf8_lossy(&value)),
                    None => println!("(nil)"),
                }
            }
            "DEL" | "DELETE" => {
                if tokens.len() != 2 {
                    eprintln!("usage: DEL <key>");
                    continue;
                }
                db.delete(tokens[1].to_string())?;
                println!("ok");
            }
            "SYNC" => {
                db.sync()?;
                println!("ok");
            }
            "CUR" => {
                println!("{current_db_path}");
            }
            "CONNECT" => {
                if tokens.len() < 2 {
                    eprintln!("usage: CONNECT <path>");
                    continue;
                }

                let next_path = tokens[1..].join(" ");
                match Db::open(&next_path) {
                    Ok(next_db) => {
                        db = next_db;
                        current_db_path = next_path;
                        println!("ok");
                    }
                    Err(err) => eprintln!("connect failed: {err}"),
                }
            }
            "NEW" => {
                if tokens.len() != 2 {
                    eprintln!("usage: NEW <name>");
                    continue;
                }

                let name = tokens[1].trim();
                if name.is_empty() {
                    eprintln!("usage: NEW <name>");
                    continue;
                }

                let new_path = PathBuf::from(DEFAULT_DB_ROOT).join(name);
                let new_path_str = new_path.to_string_lossy().to_string();
                match Db::open(&new_path_str) {
                    Ok(next_db) => {
                        db = next_db;
                        current_db_path = new_path_str;
                        println!("ok");
                    }
                    Err(err) => eprintln!("new failed: {err}"),
                }
            }
            "HELP" => {
                println!("PUT <key> <value>");
                println!("GET <key>");
                println!("DEL <key>");
                println!("SYNC");
                println!("CUR");
                println!("CONNECT <path>");
                println!("NEW <name>");
                println!("EXIT");
            }
            "EXIT" | "QUIT" => break,
            _ => eprintln!("unknown command: {}", tokens[0]),
        }
    }

    Ok(())
}

fn run_command(db_path: &str, command: Command) -> Result<(), Box<dyn std::error::Error>> {
    let mut db = Db::open(db_path)?;
    match command {
        Command::Put { key, value } => {
            db.put(key, value.into_bytes())?;
            println!("ok");
        }
        Command::Get { key } => match db.get(&key)? {
            Some(value) => println!("{}", String::from_utf8_lossy(&value)),
            None => println!("(nil)"),
        },
        Command::Delete { key } => {
            db.delete(key)?;
            println!("ok");
        }
        Command::Sync => {
            db.sync()?;
            println!("ok");
        }
    }
    Ok(())
}
