//! Standalone launcher for the AI utility (used by the stdio integration test).
//! In the product the same code runs as `browser --ai-utility`.

fn main() {
    std::process::exit(ai_utility::run_process());
}
