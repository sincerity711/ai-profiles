// SPDX-License-Identifier: MIT

// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ai_profiles_lib::cli::{invocation, open_profile, Invocation};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some(profile_shim::PROXY_PREPARE_FLAG) {
        let result = args
            .get(2)
            .ok_or_else(|| "Missing profile id".to_string())
            .and_then(|id| ai_profiles_lib::proxy::prepare_profile_launch(id));
        if let Err(message) = result {
            eprintln!("ai-profiles: {message}");
            std::process::exit(1);
        }
        return;
    }
    if args.get(1).map(String::as_str) == Some("--launch-profile-direct") {
        let result = args
            .get(2)
            .ok_or_else(|| "Missing profile id".to_string())
            .and_then(|id| ai_profiles_lib::cli::launch_profile_direct(id));
        if let Err(message) = result {
            eprintln!("ai-profiles: {message}");
            std::process::exit(1);
        }
        return;
    }
    match invocation(&args) {
        Invocation::Gui => ai_profiles_lib::run(),
        Invocation::OpenProfile(id) => {
            if let Err(message) = open_profile(id) {
                eprintln!("ai-profiles: {message}");
                std::process::exit(1);
            }
        }
        Invocation::Misuse(message) => {
            eprintln!("ai-profiles: {message}");
            std::process::exit(2);
        }
    }
}
