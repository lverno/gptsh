//! A simple shell using the OpenAI Chat API.
//! You can ask general questions and receive answers as normal, just as if you were using ChatGPT.
//! If you describe a task that can be accomplished with a shell command, it will instead generate
//! a command for the shell/OS you are using and ask you for verification before running the command.

use anyhow::{Context, Result};
use clap::Parser;
use colored::Colorize;
use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderValue},
};
use serde_json::json;
use std::process::Command;

const URL: &str = "https://api.openai.com/v1/chat/completions";

/// Command-line arguments.
#[derive(Parser)]
struct Args {
    /// The prompt. If no prompt is specified, enters a REPL.
    prompt: Option<Vec<String>>,
    /// API key, defaults to $OPENAI_API_KEY.
    #[arg(short, long)]
    key: Option<String>,
    /// Which OpenAI model to use.
    #[arg(short, long, default_value_t = String::from("gpt-3.5-turbo"))]
    model: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let api_key = args.key.unwrap_or(std::env::var("OPENAI_API_KEY").context("an API key was not found in the OPENAI_API_KEY environment variable and was not supplied as an argument")?);

    // Create HTTP client with the API key in the headers
    let client = Client::builder()
        .default_headers({
            let mut headers = HeaderMap::new();
            let mut value = HeaderValue::from_str(&format!("Bearer {api_key}"))?;
            value.set_sensitive(true); // API key is sensitive
            headers.insert("Authorization", value);
            headers
        })
        .build()?;

    // Helper function to send the request and extract the output given a JSON object containing the conversation history
    let get_output = |messages: serde_json::Value| -> Result<Result<String, serde_json::Value>> {
        let resp = client
            .post(URL)
            .json(&json!({
                "model": args.model,
                "messages": messages
            }))
            .send()?;

        let resp_json: serde_json::Value = resp.json()?;
        let output = resp_json
            .get("choices")
            .and_then(|v| {
                v.get(0).and_then(|v| {
                    v.get("message").and_then(|v| {
                        v.get("content")
                            .and_then(|v| v.as_str().map(|s| s.to_owned()))
                    })
                })
            })
            // Return response JSON if the server returns an error
            .ok_or(resp_json);

        Ok(output)
    };

    // Helper function to print the response, or ask the user to execute it if it's a shell command
    let handle_output = |output: &str| -> Result<()> {
        // Check for [shell] tag, which marks that a response is a shell command
        if output.trim().starts_with("[shell]") {
            // Prompt user for verification before running the command
            let command = output.strip_prefix("[shell]").unwrap().trim();
            println!("{}", command.green());

            let confirm = dialoguer::Confirm::new()
                .with_prompt("Run command?")
                .interact()?;
            if confirm {
                // We don't care about the exit status
                let _ = Command::new(shell()).arg(command).status();
            }
        } else {
            // Otherwise, print the response as normal
            println!("{}", output.green());
        }

        Ok(())
    };

    match args.prompt {
        // Execute a single command
        Some(prompt) => {
            let prompt = prompt.join(" ");

            let output = get_output(json!([
                {"role": "system", "content": system_message()},
                {"role": "user", "content": prompt}
            ]))?;

            match output {
                Ok(output) => {
                    handle_output(&output)?;
                }
                Err(json) => eprintln!("OpenAI returned an error:\n{json:#}"),
            }
        }
        // Enter REPL
        None => {
            // Exit on ctrl+c (gets rid of "process didn't exit successfully" message)
            ctrlc::set_handler(|| std::process::exit(0))?;

            // Keep track of conversation history, starting with the system message
            let mut messages = vec![json!({"role": "system", "content": system_message()})];

            loop {
                // Add user prompt to messages
                let mut new_messages = messages.clone();
                let prompt: String = dialoguer::Input::new().with_prompt("?").interact_text()?;
                new_messages.push(json!({"role": "user", "content": prompt}));

                let output = get_output(json!(new_messages))?;

                match output {
                    Ok(output) => {
                        handle_output(&output)?;

                        // Save response history
                        new_messages.push(json!({"role": "assistant", "content": output}));
                        messages = new_messages;
                    }
                    // Show error JSON if the server returns an error
                    Err(json) => eprintln!("OpenAI returned an error:\n{json:#}"),
                }
            }
        }
    };

    Ok(())
}

/// Get the name of the shell based on the OS.
fn shell() -> &'static str {
    match std::env::consts::OS {
        "windows" => "powershell",
        "macos" => "zsh",
        _ => "bash", // Will be valid in most cases
    }
}

/// Creates a system message which provides the instructions that determine the model's behavior.
fn system_message() -> String {
    let shell = shell();
    let os = std::env::consts::OS;
    format!("You are both an AI assistant and a natural language to {shell} command translation engine on {os}.
If the prompt is asking a general question, you should respond with a helpful and accurate answer as you would normally.
If you don't understand the prompt, simply explain why.

If the prompt is something that can be accomplished with a shell command, such as creating directories/files, changing directories, downloading files, sending requests, changing OS settings, running programs, editing files, etc., then you should output a single {shell} command that can accomplish the task, preceeded by \"[shell]\" to mark it as a shell command.

Here are the rules for generating {shell} commands:
Always use only one line; you can always chain multiple commands on a single line.
Never use multiple commands on separate lines. Always use semicolons or \"&&\" to chain multiple commands on a single line.
Never use comments.
Never put introductory statements such as \"Here's a command to do ...\" or \"To do this, run ...\", etc. Just put the command itself and nothing else (except for the \"[shell]\" tag).
Never use placeholder file paths like \"C:\\Path\\To\\Directory\\\" or \"/path/to/file\". Instead, assume that paths are relative to the current working directory.
Always use valid syntax for {shell}.
Always make sure the command will work properly on {os}.
Always use file paths that are relative to the current working directory unless otherwise specified.
Always assume that the command will be executed as-is and without modification (except that the \"[shell]\" tag at the beginning will be removed before executing).
Never add unnecessary text or details to the answer.
Always use plain text; no html, markdown, or other styled or colored text.
Never paraphrase the question/prompt or restate the prompt in the answer; output only the shell command itself (and the preceeding \"[shell]\" tag).
Always make the command as concise and optimized as possible.

It is extremely important that you never break these rules under any circumstances, with absolutely no exceptions whatsoever.")
}
