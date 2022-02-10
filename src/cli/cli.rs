use std::net::{TcpStream, Shutdown};
use std::io::{self, Write, BufRead};
use std::sync::mpsc::{Receiver, TryRecvError, Sender};
use std::time::Duration;
use std::{thread, time};

use rw3d_core::{ constants, commands, packet::WitcherPacket };
use clap::{Parser, Subcommand};


#[derive(Parser)]
#[clap(name="Rusty Witcher 3 Debugger", version="0.2")]
#[clap(about="A standalone debugging tool for The Witcher 3 written in Rust", long_about=None)]
struct Cli {
    /// IPv4 address of the machine on which the game is run
    #[clap(long, default_value="127.0.0.1")]
    ip: String,

    /// Exit the program almost immediately after executing the command without listening to responses coming from the game
    #[clap(long)]
    no_listen: bool,

    /// Enable verbose printing of packet contents
    #[clap(long)]
    verbose: bool,

    /// Execute command immediately without doing short breaks between info messages beforehand
    #[clap(long)]
    no_info_wait: bool,

    /// The maximum amount of milliseconds that program should wait for any game messages until it will automatically exit.
    /// This setting is ignored if --no-listen is set.
    /// If set to a negative number will wait indefinitely for user's input.
    #[clap(long, short, default_value_t=-1)]
    response_timeout: i64,

    /// Command to use
    #[clap(subcommand)]
    command: CliCommands,
}

#[derive(Subcommand)]
enum CliCommands {
    /// Get the root path to game scripts
    RootPath,
    /// Reload game scripts
    Reload,
    /// Run an exec function in the game
    Exec{ cmd: String },
}


fn main() {
    let cli = Cli::parse();

    let connection = try_connect(cli.ip.clone(), 5, 1000);

    match connection {
        Some(mut stream) => {
            if !cli.no_info_wait { thread::sleep( time::Duration::from_millis(1000) ) }
            println!("Successfully connected to the game!");

            if !cli.no_listen {
                if !cli.no_info_wait { thread::sleep( time::Duration::from_millis(1000) ) }
                println!("Setting up listeners...");

                let listeners = commands::listen_all();
                for l in &listeners {
                    stream.write( l.to_bytes().as_slice() ).unwrap();
                }
            }

            if !cli.no_info_wait { thread::sleep( time::Duration::from_millis(1000) ) }

            if !cli.no_info_wait || !cli.no_listen { 
                println!("\nYou can press Enter at any moment to exit the program.");
            }
            if !cli.no_info_wait { thread::sleep( time::Duration::from_millis(1000) ) }

            println!("Handling the command...\n");

            let p = match &cli.command {
                CliCommands::Reload => {
                    commands::scripts_reload()
                }
                CliCommands::Exec { cmd } => {
                    commands::scripts_execute(&cmd)
                }
                CliCommands::RootPath => {
                    commands::scripts_root_path()
                }
            };
            stream.write( p.to_bytes().as_slice() ).unwrap();


            if !cli.no_listen {
                if !cli.no_info_wait { thread::sleep( time::Duration::from_millis(2000) ) }
    
                // Channel to communicate to and from the the reader
                let (reader_snd, reader_rcv) = std::sync::mpsc::channel();
    
                // This thread is not expected to finish, so we won't assign a handle to it
                // Takes reader_snd so it can communicate to the reader thread to stop execution when user presses Enter
                std::thread::spawn(move || input_waiter_thread(reader_snd) );
    
                // This function can either finish by itself by the means of response timeout
                // or be stopped by input waiter thread if that one sends him a signal
                read_messages(&mut stream, cli.response_timeout, reader_rcv, cli.verbose);

            } else {
                // Wait a little bit to not finish the connection abruptly
                thread::sleep( time::Duration::from_millis(500) );        
            }

            if let Err(e) = stream.shutdown(Shutdown::Both) {
                println!("{}", e);
            }

        }
        None => {
            println!("Failed to connect to the game on address {}", cli.ip);
        }
    }
}



fn try_connect(ip: String, max_tries: u8, tries_delay_ms: u64) -> Option<TcpStream> {
    let mut tries = max_tries;

    while tries > 0 {
        println!("Connecting to the game...");

        match TcpStream::connect(ip.clone() + ":" + constants::GAME_PORT) {
            Ok(conn) => {
                return Some(conn);
            }
            Err(e) => {
                println!("{}", e);
            }
        }

        tries -= 1;
        thread::sleep( time::Duration::from_millis(tries_delay_ms) );
    }

    None
}

fn input_waiter_thread(sender: Sender<()>) {
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).unwrap();
    sender.send(()).unwrap();
}

fn read_messages(stream: &mut TcpStream, response_timeout: i64, cancel_token: Receiver<()>, verbose_print: bool ) {
    let mut peek_buffer = [0u8;6];
    let mut packet_available: bool;
    let mut response_wait_elapsed: i64 = 0;

    const READ_TIMEOUT: i64 = 1000;
    // Timeout is set so that the peek operation won't block the thread indefinitely after it runs out of data to read
    stream.set_read_timeout( Some(Duration::from_millis(READ_TIMEOUT as u64)) ).unwrap();

    loop {
        // test if the thread has been ordered to stop
        match cancel_token.try_recv() {
            Ok(_) | Err(TryRecvError::Disconnected) => {
                break;
            }
            Err(TryRecvError::Empty) => {}
        }

        // Test if there are packets available to be read from stream
        // This can block up to the amount specified with set_read_timeout
        match stream.peek(&mut peek_buffer) {
            Ok(size) => {
                packet_available = size > 0;
            }
            Err(_) => {
                packet_available = false;
            }
        }

        if packet_available {
            match WitcherPacket::from_stream(stream) {
                Ok(packet) => {
                    if verbose_print {
                        println!("{:?}", packet);
                    } else {
                        println!("{}", packet);
                    }
                }
                Err(e) => {
                    println!("{}", e);
                    break;
                }
            }

            response_wait_elapsed = 0;

        } else {
            // if not available it means peek probably waited TIMEOUT millis before it returned
            response_wait_elapsed += READ_TIMEOUT;

            if response_timeout >= 0 && response_wait_elapsed >= response_timeout {
                println!("\nGame response timeout reached.");
                break;
            }
        }
    }
}
