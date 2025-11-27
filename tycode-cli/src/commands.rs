use crate::state::State;

pub enum LocalCommandResult {
    Handled {
        msg: String,
    },

    /// A command to exit the app was detected
    Exit,

    /// The command was not processed locally (and should be sent to the actor).
    Unhandled,
}

pub fn handle_local_command(state: &mut State, input: &str) -> LocalCommandResult {
    match input.trim() {
        "/timing" => {
            state.show_timing = !state.show_timing;
            LocalCommandResult::Handled {
                msg: format!(
                    "Timings: {}",
                    if state.show_timing {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ),
            }
        }
        "/verbose" => {
            state.show_reasoning = !state.show_reasoning;
            LocalCommandResult::Handled {
                msg: format!(
                    "Verbose mode: {} (showing model reasoning)",
                    if state.show_reasoning {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ),
            }
        }
        "/exit" | "/quit" => LocalCommandResult::Exit,
        _ => LocalCommandResult::Unhandled,
    }
}
