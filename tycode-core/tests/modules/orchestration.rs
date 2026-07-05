//! Orchestration workflow simulation tests.
//!
//! Drives the mechanical builder and swarm agents end-to-end through the chat
//! actor with a scripted mock provider, validating that workflow phase
//! transitions produce the expected agent sequence.

#[path = "../fixture.rs"]
mod fixture;

use fixture::MockBehavior;
use tycode_core::chat::events::ChatEvent;
use tycode_core::orchestration::events::{
    AgentOrigin, OrchestrationEvent, OrchestrationPayload, OutcomeStatus, ReviewVerdict,
    WorkflowPhase,
};

fn complete_task(result: &str) -> MockBehavior {
    MockBehavior::ToolUse {
        tool_name: "complete_task".to_string(),
        tool_arguments: serde_json::json!({ "result": result, "success": true }).to_string(),
    }
}

fn fail_task(result: &str) -> MockBehavior {
    MockBehavior::ToolUse {
        tool_name: "complete_task".to_string(),
        tool_arguments: serde_json::json!({ "result": result, "success": false }).to_string(),
    }
}

fn all_event_text(events: &[ChatEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            ChatEvent::MessageAdded(message) => Some(message.content.clone()),
            ChatEvent::Error(message) => Some(message.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn orchestration_events(events: &[ChatEvent]) -> Vec<OrchestrationEvent> {
    events
        .iter()
        .filter_map(|event| match event {
            ChatEvent::Orchestration(event) => Some(event.clone()),
            _ => None,
        })
        .collect()
}

fn phases(events: &[OrchestrationEvent]) -> Vec<WorkflowPhase> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            OrchestrationPayload::PhaseChanged { phase } => Some(phase.clone()),
            _ => None,
        })
        .collect()
}

fn started_agent_types(events: &[OrchestrationEvent]) -> Vec<&str> {
    events
        .iter()
        .filter(|event| matches!(&event.payload, OrchestrationPayload::AgentStarted { .. }))
        .map(|event| event.agent_type.as_str())
        .collect()
}

fn completed_agent_types(events: &[OrchestrationEvent]) -> Vec<&str> {
    events
        .iter()
        .filter(|event| matches!(&event.payload, OrchestrationPayload::AgentCompleted { .. }))
        .map(|event| event.agent_type.as_str())
        .collect()
}

#[test]
fn test_builder_runs_plan_implement_review_pipeline() {
    fixture::run_with_agent("builder", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                complete_task("THE PLAN: add a widget to widgets.rs"),
                complete_task("implemented the widget"),
                complete_task("review approved"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("add a widget").await;

        let requests = fixture.get_all_ai_requests();
        assert_eq!(
            requests.len(),
            3,
            "expected exactly planner + coder + review requests, got {}",
            requests.len()
        );

        assert!(
            requests[0].system_prompt.contains("PLANNER"),
            "first request should go to the planner"
        );
        assert!(
            requests[1]
                .system_prompt
                .contains("executing assigned coding tasks"),
            "second request should go to the coder"
        );
        let coder_input: String = requests[1]
            .messages
            .iter()
            .map(|m| m.content.text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            coder_input.contains("THE PLAN"),
            "coder must receive the plan. Input: {coder_input}"
        );
        assert!(
            requests[2].system_prompt.contains("review sub-agent"),
            "third request should go to the review agent"
        );

        let text = all_event_text(&events);
        assert!(
            text.contains("Task completed [success=true]"),
            "builder completion should cascade to the root. Events: {text}"
        );
        assert!(
            text.contains("implemented the widget") && text.contains("review approved"),
            "final result should carry implementation and review. Events: {text}"
        );

        // Structured stream: the whole pipeline must be renderable from typed
        // events alone.
        let structured = orchestration_events(&events);
        assert_eq!(
            started_agent_types(&structured),
            vec!["builder", "planner", "coder", "review"],
            "the root is announced before its first child, then every spawn"
        );
        assert_eq!(
            completed_agent_types(&structured),
            vec!["planner", "coder", "review"],
            "every pop must emit AgentCompleted"
        );
        assert_eq!(
            phases(&structured),
            vec![
                WorkflowPhase::BuilderPlanning,
                WorkflowPhase::BuilderImplementing,
                WorkflowPhase::BuilderReviewing { round: 1 },
            ],
            "phase transitions must be emitted in order"
        );
        let root_id = structured
            .iter()
            .find_map(|event| match &event.payload {
                OrchestrationPayload::AgentStarted {
                    origin: AgentOrigin::Root,
                    parent_agent_id,
                    depth,
                    ..
                } => {
                    assert!(parent_agent_id.is_none(), "the root has no parent");
                    assert_eq!(*depth, 1);
                    Some(event.agent_id.clone())
                }
                _ => None,
            })
            .expect("the root builder must be announced");
        for event in &structured {
            if let OrchestrationPayload::AgentStarted {
                parent_agent_id,
                origin: AgentOrigin::Workflow,
                depth,
                interactive,
                ..
            } = &event.payload
            {
                assert_eq!(
                    parent_agent_id.as_deref(),
                    Some(root_id.as_str()),
                    "workflow spawns must resolve to the announced root"
                );
                assert_eq!(*depth, 2, "builder children sit directly on the root");
                assert!(interactive);
            }
        }
        assert!(
            structured.iter().any(|event| matches!(
                &event.payload,
                OrchestrationPayload::ReviewRoundResolved {
                    round: 1,
                    verdict: ReviewVerdict::Approved,
                    ..
                }
            )),
            "the approved review must emit a typed verdict"
        );
    });
}

#[test]
fn test_builder_planning_failure_fails_fast() {
    fixture::run_with_agent("builder", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                fail_task("cannot plan: repository is empty"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("add a widget").await;

        let requests = fixture.get_all_ai_requests();
        assert_eq!(
            requests.len(),
            1,
            "planning failure must not spawn a coder, got {} requests",
            requests.len()
        );

        let text = all_event_text(&events);
        assert!(
            text.contains("Task completed [success=false]") && text.contains("Planning failed"),
            "builder should fail fast on planning failure. Events: {text}"
        );
    });
}

const SWARM_PLAN: &str = r#"## Plan
Split the change across two files.

```json
{"assignments": [
  {"file": "src/a.rs", "instructions": "define the struct", "shared_surfaces": ["pub struct X { y: u32 }"]},
  {"file": "src/b.rs", "instructions": "consume the struct", "shared_surfaces": ["pub struct X { y: u32 }"]}
]}
```"#;

#[test]
fn test_swarm_fans_out_workers_and_integration_review() {
    fixture::run_with_agent("swarm", |mut fixture| async move {
        // Queue: planner, then 4 fan-out completions (2 workers + 2 pair
        // reviews, order nondeterministic under concurrency so all
        // identical), then the integration review.
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                complete_task(SWARM_PLAN),
                complete_task("worker done"),
                complete_task("worker done"),
                complete_task("worker done"),
                complete_task("worker done"),
                complete_task("integration review approved"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("make the wide change").await;

        let requests = fixture.get_all_ai_requests();
        assert_eq!(
            requests.len(),
            6,
            "expected planner + 2 workers + 2 pair reviews + integration review, got {}",
            requests.len()
        );

        let text = all_event_text(&events);
        assert!(
            text.contains("Fan-out: launching 2 worker(s)"),
            "swarm should fan out two workers. Events: {text}"
        );
        assert!(
            text.contains("Task completed [success=true]") && text.contains("Swarm complete"),
            "swarm should complete after integration approval. Events: {text}"
        );

        let structured = orchestration_events(&events);
        let fanout_started = structured
            .iter()
            .find_map(|event| match &event.payload {
                OrchestrationPayload::FanOutStarted {
                    fanout_id,
                    total,
                    workers,
                    ..
                } => Some((fanout_id.clone(), *total, workers.clone())),
                _ => None,
            })
            .expect("fan-out must announce itself");
        let (fanout_id, total, workers) = fanout_started;
        assert_eq!(total, 2);
        let labels: Vec<&str> = workers.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, vec!["src/a.rs", "src/b.rs"]);
        assert!(workers
            .iter()
            .all(|w| w.reviewed && w.agent_type == "file_impl"));

        let started: Vec<String> = structured
            .iter()
            .filter_map(|event| match &event.payload {
                OrchestrationPayload::WorkerStarted {
                    fanout_id: id,
                    worker_id,
                    ..
                } if *id == fanout_id => Some(worker_id.clone()),
                _ => None,
            })
            .collect();
        let completed: Vec<(String, OutcomeStatus)> = structured
            .iter()
            .filter_map(|event| match &event.payload {
                OrchestrationPayload::WorkerCompleted {
                    fanout_id: id,
                    worker_id,
                    status,
                    ..
                } if *id == fanout_id => Some((worker_id.clone(), *status)),
                _ => None,
            })
            .collect();
        assert_eq!(started.len(), 2, "each worker must emit WorkerStarted");
        assert_eq!(completed.len(), 2, "each worker must emit WorkerCompleted");
        for (worker_id, status) in &completed {
            assert!(started.contains(worker_id), "worker ids must be stable");
            assert_eq!(*status, OutcomeStatus::Succeeded);
        }
        assert!(structured.iter().any(|event| matches!(
            &event.payload,
            OrchestrationPayload::FanOutCompleted {
                fanout_id: id,
                status: OutcomeStatus::Succeeded,
            } if *id == fanout_id
        )));
        assert!(
            structured.iter().any(|event| matches!(
                &event.payload,
                OrchestrationPayload::PlanSelected { candidate: None }
            )),
            "single-model planning must still emit PlanSelected"
        );
    });
}

#[test]
fn test_swarm_consensus_elimination_round_still_converges() {
    fixture::run_with_agent("swarm", |mut fixture| async move {
        fixture
            .update_settings(|settings| {
                settings.swarm_models = vec![
                    tycode_core::ai::model::Model::None,
                    tycode_core::ai::model::Model::None,
                ];
            })
            .await;

        // Round 1 splits: one panelist approves plan:1 and one revises, both
        // vote plan:2 worst. That eliminates plan:2's seat, leaving a single
        // survivor whose plan proceeds — either the original or the revision,
        // both parallelizable, so the request count is deterministic even
        // though behavior-to-panelist assignment is not.
        let revision = format!("Revised: merged the best ideas.\n{SWARM_PLAN}\nWORST: plan:2:None");
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                complete_task(SWARM_PLAN),
                complete_task(SWARM_PLAN),
                complete_task("APPROVE: plan:1:None\nWORST: plan:2:None"),
                complete_task(&revision),
                complete_task("worker done"),
                complete_task("worker done"),
                complete_task("worker done"),
                complete_task("worker done"),
                complete_task("integration review approved"),
                complete_task("integration review approved"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("wide change with a disputed plan").await;

        let requests = fixture.get_all_ai_requests();
        assert_eq!(
            requests.len(),
            10,
            "expected 2 planners + 2 panelists + 2 workers + 2 pair reviews + 2 integration reviews, got {}",
            requests.len()
        );

        let text = all_event_text(&events);
        assert!(
            text.contains("Task completed [success=true]") && text.contains("Swarm complete"),
            "elimination round should still converge to completion. Events: {text}"
        );

        let structured = orchestration_events(&events);
        let (eliminated, remaining) = structured
            .iter()
            .find_map(|event| match &event.payload {
                OrchestrationPayload::ConsensusRoundResolved {
                    round: 1,
                    eliminated: Some(eliminated),
                    remaining,
                    ..
                } => Some((eliminated.clone(), remaining.clone())),
                _ => None,
            })
            .expect("the elimination round must emit a typed resolution");
        assert!(
            eliminated.label.starts_with("plan:2:None"),
            "both panelists voted plan:2 worst; got {}",
            eliminated.label
        );
        assert_eq!(remaining.len(), 1);
        assert!(structured.iter().any(|event| matches!(
            &event.payload,
            OrchestrationPayload::PlanSelected { candidate: Some(candidate) }
                if candidate.label.starts_with("plan:1:None")
        )));
    });
}

#[test]
fn test_swarm_consensus_runs_multi_model_pipeline() {
    fixture::run_with_agent("swarm", |mut fixture| async move {
        fixture
            .update_settings(|settings| {
                // The mock provider only supports Model::None; a two-entry
                // roster still exercises the full consensus pipeline.
                settings.swarm_models = vec![
                    tycode_core::ai::model::Model::None,
                    tycode_core::ai::model::Model::None,
                ];
            })
            .await;

        // Phases are barriers, so queue order is deterministic per phase even
        // though worker order within a phase is not: 2 planners, 2 judges,
        // 2 workers + 2 pair reviews, 2 integration reviews.
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                complete_task(SWARM_PLAN),
                complete_task(SWARM_PLAN),
                complete_task("plan:1:None"),
                complete_task("plan:1:None"),
                complete_task("worker done"),
                complete_task("worker done"),
                complete_task("worker done"),
                complete_task("worker done"),
                complete_task("integration review approved"),
                complete_task("integration review approved"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("make the wide change with consensus").await;

        let requests = fixture.get_all_ai_requests();
        assert_eq!(
            requests.len(),
            10,
            "expected 2 planners + 2 judges + 2 workers + 2 pair reviews + 2 integration reviews, got {}",
            requests.len()
        );

        let judge_requests = requests
            .iter()
            .filter(|request| request.system_prompt.contains("planning panel"))
            .count();
        assert_eq!(judge_requests, 2, "every roster model must vote");

        let text = all_event_text(&events);
        assert!(
            text.contains("Task completed [success=true]") && text.contains("Swarm complete"),
            "consensus swarm should complete after all models approve. Events: {text}"
        );

        let structured = orchestration_events(&events);
        assert!(
            phases(&structured).iter().any(|phase| matches!(
                phase,
                WorkflowPhase::SwarmConsensus { round: 1, candidates } if candidates.len() == 2
            )),
            "entering the tournament must announce the candidates"
        );
        assert!(
            structured.iter().any(|event| matches!(
                &event.payload,
                OrchestrationPayload::ConsensusRoundResolved {
                    round: 1,
                    eliminated: None,
                    verdicts,
                    ..
                } if verdicts.len() == 2
            )),
            "unanimous approval must emit a round resolution with both verdicts"
        );
        assert!(structured.iter().any(|event| matches!(
            &event.payload,
            OrchestrationPayload::PlanSelected { candidate: Some(candidate) }
                if candidate.label == "plan:1:None"
        )));
    });
}

#[test]
fn test_spawn_agent_tool_emits_started_and_completed_events() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "coder", "task": "write a file"}"#
                        .to_string(),
                },
                complete_task("done"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("delegate this").await;

        let structured = orchestration_events(&events);
        let root = structured
            .iter()
            .find(|event| {
                matches!(
                    &event.payload,
                    OrchestrationPayload::AgentStarted {
                        origin: AgentOrigin::Root,
                        ..
                    }
                )
            })
            .expect("the root must be announced before its first child");
        assert_eq!(root.agent_type, "coordinator");
        let started = structured
            .iter()
            .find(|event| {
                event.agent_type == "coder"
                    && matches!(&event.payload, OrchestrationPayload::AgentStarted { .. })
            })
            .expect("spawn_agent must emit AgentStarted");
        let OrchestrationPayload::AgentStarted {
            parent_agent_id,
            task_preview,
            origin,
            depth,
            interactive,
            ..
        } = &started.payload
        else {
            unreachable!();
        };
        assert_eq!(
            parent_agent_id.as_deref(),
            Some(root.agent_id.as_str()),
            "the child's parent must be the announced root"
        );
        assert_eq!(task_preview, "write a file");
        assert!(
            matches!(origin, AgentOrigin::Tool { tool_call_id } if !tool_call_id.is_empty()),
            "tool spawns must carry the spawning tool_call_id"
        );
        assert_eq!(*depth, 2);
        assert!(interactive);

        let completed = structured
            .iter()
            .find(|event| matches!(&event.payload, OrchestrationPayload::AgentCompleted { .. }))
            .expect("complete_task must emit AgentCompleted");
        assert_eq!(completed.agent_id, started.agent_id, "ids must be stable");
        assert!(matches!(
            &completed.payload,
            OrchestrationPayload::AgentCompleted {
                status: OutcomeStatus::Succeeded,
                result,
            } if result == "done"
        ));
    });
}

#[test]
fn test_plain_chat_emits_no_orchestration_events() {
    fixture::run(|mut fixture: fixture::Fixture| async move {
        fixture.set_mock_behavior(MockBehavior::Success);

        let events = fixture.step("just answer a question").await;

        assert!(
            orchestration_events(&events).is_empty(),
            "ordinary chat must keep the stream free of orchestration events"
        );
    });
}

#[test]
fn test_progress_messages_can_be_suppressed_for_structured_consumers() {
    fixture::run_with_agent("builder", |mut fixture| async move {
        fixture
            .update_settings(|settings| {
                settings.orchestration_progress_messages = false;
            })
            .await;
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                complete_task("THE PLAN: add a widget"),
                complete_task("implemented"),
                complete_task("review approved"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("add a widget").await;

        let text = all_event_text(&events);
        assert!(
            !text.contains("🔄") && !text.contains("⚡") && !text.contains("🔍"),
            "progress strings must be suppressed. Events: {text}"
        );
        assert!(
            text.contains("Task completed [success=true]"),
            "the final result message must survive suppression. Events: {text}"
        );

        let structured = orchestration_events(&events);
        assert_eq!(
            started_agent_types(&structured),
            vec!["builder", "planner", "coder", "review"],
            "structured events must be unaffected by suppression"
        );
    });
}

#[test]
fn test_swarm_degrades_to_sequential_coder_without_assignments() {
    fixture::run_with_agent("swarm", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                complete_task("a plan with no assignment block"),
                complete_task("implemented sequentially"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("small change").await;

        let requests = fixture.get_all_ai_requests();
        assert_eq!(
            requests.len(),
            2,
            "unparseable plan should degrade to planner + single coder, got {}",
            requests.len()
        );
        assert!(
            requests[1]
                .system_prompt
                .contains("executing assigned coding tasks"),
            "degraded path should use the coder agent"
        );

        let text = all_event_text(&events);
        assert!(
            text.contains("Task completed [success=true]"),
            "degraded swarm should still complete. Events: {text}"
        );
    });
}

/// Drains events after a non-step actor message (e.g. ResumeSession) until
/// the turn's TypingStatusChanged(false), mirroring `Session::step`.
async fn drain_turn(fixture: &mut fixture::Session) -> Vec<ChatEvent> {
    let mut events = Vec::new();
    while let Some(event) = fixture.event_rx.recv().await {
        if matches!(event, ChatEvent::TypingStatusChanged(false)) {
            break;
        }
        if !matches!(event, ChatEvent::TypingStatusChanged(_)) {
            events.push(event);
        }
    }
    events
}

#[test]
fn test_clear_aborts_sub_agents_before_conversation_reset() {
    fixture::run_with_agent("tycode", |mut fixture| async move {
        // Leave a sub-agent live on the stack: the spawned one_shot asks the
        // user a question, which stops the turn without completing the agent.
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "one_shot", "task": "hold the stack"}"#
                        .to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "ask_user_question".to_string(),
                    tool_arguments: r#"{"question": "should I proceed?"}"#.to_string(),
                },
                MockBehavior::Success,
            ],
        });
        fixture.step("delegate and pause").await;

        let events = fixture.step("/clear").await;

        let aborted_index = events
            .iter()
            .position(|event| match event {
                ChatEvent::Orchestration(inner) => matches!(
                    &inner.payload,
                    OrchestrationPayload::AgentCompleted {
                        status: OutcomeStatus::Aborted,
                        ..
                    }
                ),
                _ => false,
            })
            .expect("/clear with a live sub-agent must emit an Aborted completion");
        let cleared_index = events
            .iter()
            .position(|event| matches!(event, ChatEvent::ConversationCleared))
            .expect("/clear must emit ConversationCleared");
        assert!(
            aborted_index < cleared_index,
            "Aborted completions must land before the conversation reset so \
             consumers close the old tree first. Aborted at {aborted_index}, \
             cleared at {cleared_index}"
        );
    });
}

#[test]
fn test_resume_session_reannounces_root_for_new_orchestration() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "coder", "task": "first task"}"#.to_string(),
                },
                complete_task("first done"),
                MockBehavior::Success,
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "coder", "task": "second task"}"#.to_string(),
                },
                complete_task("second done"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("delegate this").await;
        let session_id = events
            .iter()
            .find_map(|event| match event {
                ChatEvent::SessionStarted { session_id } => Some(session_id.clone()),
                _ => None,
            })
            .expect("the actor announces its session id");

        fixture
            .actor
            .tx
            .send(tycode_core::chat::ChatActorMessage::ResumeSession { session_id })
            .unwrap();
        let resume_events = drain_turn(&mut fixture).await;
        assert!(
            resume_events
                .iter()
                .any(|event| matches!(event, ChatEvent::ConversationCleared)),
            "resume must reset the consumer's view"
        );

        // New live orchestration after the resume: the root must be announced
        // again (the reset discarded the earlier announcement) and the new
        // child must attach to it.
        let events = fixture.step("delegate again").await;
        let structured = orchestration_events(&events);
        let root = structured
            .iter()
            .find(|event| {
                matches!(
                    &event.payload,
                    OrchestrationPayload::AgentStarted {
                        origin: AgentOrigin::Root,
                        ..
                    }
                )
            })
            .expect("the root must be re-announced after a session resume");
        let child_parent = structured
            .iter()
            .find_map(|event| match &event.payload {
                OrchestrationPayload::AgentStarted {
                    origin: AgentOrigin::Tool { .. },
                    parent_agent_id,
                    ..
                } => parent_agent_id.clone(),
                _ => None,
            })
            .expect("the new child must be announced");
        assert_eq!(
            child_parent, root.agent_id,
            "post-resume children must attach to the re-announced root"
        );
    });
}

#[test]
fn test_unsupported_model_workers_emit_started_completed_pairs() {
    fixture::run_with_agent("swarm", |mut fixture| async move {
        // The mock provider only supports Model::None, so both roster models
        // fail preflight without ever making an AI request.
        fixture
            .update_settings(|settings| {
                settings.swarm_models = vec![
                    tycode_core::ai::model::Model::ClaudeFable,
                    tycode_core::ai::model::Model::Gpt,
                ];
            })
            .await;
        fixture.set_mock_behavior(MockBehavior::Success);

        let events = fixture.step("wide change on an unsupported roster").await;

        let structured = orchestration_events(&events);
        let started: Vec<String> = structured
            .iter()
            .filter_map(|event| match &event.payload {
                OrchestrationPayload::WorkerStarted { worker_id, .. } => Some(worker_id.clone()),
                _ => None,
            })
            .collect();
        let completed: Vec<String> = structured
            .iter()
            .filter_map(|event| match &event.payload {
                OrchestrationPayload::WorkerCompleted {
                    worker_id,
                    status,
                    summary,
                    ..
                } => {
                    assert_eq!(*status, OutcomeStatus::Failed);
                    assert!(summary.contains("not available"));
                    Some(worker_id.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            started.len(),
            2,
            "preflight failures still emit WorkerStarted"
        );
        assert_eq!(completed.len(), 2);
        for worker_id in &completed {
            assert!(
                started.contains(worker_id),
                "every WorkerCompleted must pair with a WorkerStarted"
            );
        }
        assert!(structured.iter().any(|event| matches!(
            &event.payload,
            OrchestrationPayload::FanOutCompleted {
                status: OutcomeStatus::Failed,
                ..
            }
        )));

        let text = all_event_text(&events);
        assert!(
            text.contains("Task completed [success=false]")
                && text.contains("All consensus planners failed"),
            "an all-unsupported roster must fail the swarm cleanly. Events: {text}"
        );
        assert!(
            fixture.get_all_ai_requests().is_empty(),
            "preflight failures must not reach the provider"
        );
    });
}

fn session_id_of(events: &[ChatEvent]) -> String {
    events
        .iter()
        .find_map(|event| match event {
            ChatEvent::SessionStarted { session_id } => Some(session_id.clone()),
            _ => None,
        })
        .expect("the actor announces its session id")
}

fn aborted_agent_ids(events: &[ChatEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            ChatEvent::Orchestration(inner) => match &inner.payload {
                OrchestrationPayload::AgentCompleted {
                    status: OutcomeStatus::Aborted,
                    ..
                } => Some(inner.agent_id.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Every persisted AgentCompleted must terminate an agent whose AgentStarted
/// is persisted earlier in the same session history: replaying a session must
/// never surface a terminal event for an agent the consumer has never seen.
fn assert_no_orphan_completions(events: &[ChatEvent]) {
    let mut started = std::collections::HashSet::new();
    for event in events {
        if let ChatEvent::Orchestration(inner) = event {
            match &inner.payload {
                OrchestrationPayload::AgentStarted { .. } => {
                    started.insert(inner.agent_id.clone());
                }
                OrchestrationPayload::AgentCompleted { .. } => {
                    assert!(
                        started.contains(&inner.agent_id),
                        "persisted AgentCompleted for agent {} has no persisted AgentStarted",
                        inner.agent_id
                    );
                }
                _ => {}
            }
        }
    }
}

fn parked_sub_agent_behaviors() -> MockBehavior {
    MockBehavior::BehaviorQueue {
        behaviors: vec![
            MockBehavior::ToolUse {
                tool_name: "spawn_agent".to_string(),
                tool_arguments: r#"{"agent_type": "one_shot", "task": "hold the stack"}"#
                    .to_string(),
            },
            MockBehavior::ToolUse {
                tool_name: "ask_user_question".to_string(),
                tool_arguments: r#"{"question": "should I proceed?"}"#.to_string(),
            },
            MockBehavior::Success,
        ],
    }
}

#[test]
fn test_resume_does_not_persist_prior_session_aborts() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");
    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        let workspace = fixture::Workspace::new();

        // Session A: a plain conversation with no orchestration.
        let mut session_a = workspace.spawn_session("tycode", fixture::MockBehavior::Success);
        let events = session_a.step("hello there").await;
        let session_a_id = session_id_of(&events);
        drop(session_a);

        // Session B: park a live sub-agent, then resume session A while the
        // child is still on the stack. The unwind's Aborted completion
        // belongs to session B's history, never to A's.
        let mut session_b = workspace.spawn_session("tycode", parked_sub_agent_behaviors());
        let events = session_b.step("delegate and pause").await;
        let session_b_id = session_id_of(&events);
        assert_ne!(session_a_id, session_b_id);

        session_b
            .actor
            .tx
            .send(tycode_core::chat::ChatActorMessage::ResumeSession {
                session_id: session_a_id.clone(),
            })
            .unwrap();
        drain_turn(&mut session_b).await;

        // A turn in the resumed session persists session A.
        session_b.step("hello again").await;

        let saved_a = tycode_core::persistence::storage::load_session(
            &session_a_id,
            Some(&workspace.sessions_dir()),
        )
        .expect("session A persists");
        assert!(
            aborted_agent_ids(&saved_a.events).is_empty(),
            "the resumed session must not inherit the departing session's Aborted events"
        );
        assert_no_orphan_completions(&saved_a.events);

        let saved_b = tycode_core::persistence::storage::load_session(
            &session_b_id,
            Some(&workspace.sessions_dir()),
        )
        .expect("session B persists");
        assert!(
            !aborted_agent_ids(&saved_b.events).is_empty(),
            "the departing session records the Aborted terminal for its own tree"
        );
        assert_no_orphan_completions(&saved_b.events);
    }));
}

#[test]
fn test_clear_persists_aborts_with_matching_started_events() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");
    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        let workspace = fixture::Workspace::new();
        let mut session = workspace.spawn_session("tycode", parked_sub_agent_behaviors());

        let events = session.step("delegate and pause").await;
        let session_id = session_id_of(&events);

        session.step("/clear").await;
        session.step("hello after the reset").await;

        let saved = tycode_core::persistence::storage::load_session(
            &session_id,
            Some(&workspace.sessions_dir()),
        )
        .expect("session persists");
        let aborted = aborted_agent_ids(&saved.events);
        assert_eq!(
            aborted.len(),
            1,
            "/clear must persist the Aborted terminal for the parked sub-agent"
        );
        assert_no_orphan_completions(&saved.events);
    }));
}

#[test]
fn test_set_root_agent_applies_before_first_message() {
    // Tyde's flow: connect (SessionStarted), pick the orchestration mode via
    // the typed SetRootAgent command, then send the first user message. The
    // first turn must run under the selected agent.
    fixture::run_with_agent("tycode", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                complete_task("THE PLAN: add a widget"),
                complete_task("implemented"),
                complete_task("review approved"),
                MockBehavior::Success,
            ],
        });

        fixture.actor.set_root_agent("builder".to_string()).unwrap();
        let events = drain_turn(&mut fixture).await;
        assert!(
            events.iter().any(|event| matches!(
                event,
                ChatEvent::RootAgentChanged { agent } if agent == "builder"
            )),
            "the switch must be acknowledged with a typed event. Events: {events:?}"
        );

        let events = fixture.step("add a widget").await;
        let requests = fixture.get_all_ai_requests();
        assert!(
            requests[0].system_prompt.contains("PLANNER"),
            "the first user turn must run the builder pipeline, starting with \
             the planner"
        );
        let text = all_event_text(&events);
        assert!(
            text.contains("Task completed [success=true]"),
            "the builder pipeline must complete. Events: {text}"
        );
    });
}

#[test]
fn test_set_root_agent_rejects_unknown_agent() {
    fixture::run_with_agent("tycode", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::Success);

        fixture
            .actor
            .set_root_agent("does_not_exist".to_string())
            .unwrap();
        let events = drain_turn(&mut fixture).await;
        assert!(
            events.iter().any(|event| matches!(
                event,
                ChatEvent::Error(message) if message.contains("Unknown agent type 'does_not_exist'")
            )),
            "unknown agents must be rejected with a typed error. Events: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ChatEvent::RootAgentChanged { .. })),
            "a rejected switch must not acknowledge a root change"
        );

        // The session still works under the original root.
        let events = fixture.step("hello").await;
        assert!(
            orchestration_events(&events).is_empty(),
            "the original conversational root must remain active"
        );
    });
}

#[test]
fn test_set_root_agent_preserves_conversation() {
    fixture::run_with_agent("tycode", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("remember the magic word is xyzzy").await;

        fixture
            .actor
            .set_root_agent("one_shot".to_string())
            .unwrap();
        drain_turn(&mut fixture).await;

        fixture.step("what was the magic word?").await;
        let request = fixture
            .get_last_ai_request()
            .expect("the switched root converses");
        let history: String = request
            .messages
            .iter()
            .map(|m| m.content.text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            history.contains("xyzzy"),
            "the root conversation must survive the agent switch"
        );
    });
}

#[test]
fn test_auto_mode_gates_swarm_mechanically() {
    fixture::run_with_agent("tycode", |mut fixture| async move {
        // Default orchestration_mode is auto: even if the model tries to
        // spawn swarm, the tool must reject it.
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "swarm", "task": "wide change"}"#.to_string(),
                },
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("make a wide change").await;

        let requests = fixture.get_all_ai_requests();
        assert!(
            requests[0]
                .system_prompt
                .contains("Orchestration Policy: auto"),
            "tycode must carry the auto policy by default"
        );
        let spawn_tool = requests[0]
            .tools
            .iter()
            .find(|tool| tool.name == "spawn_agent")
            .expect("tycode has spawn_agent");
        assert!(
            !spawn_tool.description.contains("swarm"),
            "swarm must not be offered in auto mode. Description: {}",
            spawn_tool.description
        );

        assert!(
            orchestration_events(&events).is_empty(),
            "the rejected spawn must not start any agent"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                ChatEvent::ToolExecutionCompleted { success: false, tool_result, .. }
                    if format!("{tool_result:?}").contains("not allowed")
            )),
            "the spawn must be mechanically rejected"
        );
    });
}

#[test]
fn test_swarm_mode_offers_and_spawns_swarm() {
    fixture::run_with_agent("tycode", |mut fixture| async move {
        fixture
            .update_settings(|settings| {
                settings.orchestration_mode =
                    tycode_core::settings::config::OrchestrationMode::Swarm;
            })
            .await;
        // tycode delegates to swarm; the swarm's planner degrades to a
        // single coder (no assignments block) which completes.
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "swarm", "task": "make the wide change"}"#
                        .to_string(),
                },
                complete_task("a plan with no assignment block"),
                complete_task("implemented"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("make a wide change").await;

        let requests = fixture.get_all_ai_requests();
        assert!(
            requests[0]
                .system_prompt
                .contains("Orchestration Policy: swarm (required)"),
            "tycode must carry the required-swarm policy"
        );
        let spawn_tool = requests[0]
            .tools
            .iter()
            .find(|tool| tool.name == "spawn_agent")
            .expect("tycode has spawn_agent");
        assert!(
            spawn_tool.description.contains("swarm"),
            "swarm must be offered in swarm mode"
        );

        let structured = orchestration_events(&events);
        assert!(
            structured.iter().any(|event| event.agent_type == "swarm"
                && matches!(&event.payload, OrchestrationPayload::AgentStarted { .. })),
            "the swarm workflow must actually start"
        );
    });
}

#[test]
fn test_builder_mode_requires_builder_and_gates_swarm() {
    fixture::run_with_agent("tycode", |mut fixture| async move {
        fixture
            .update_settings(|settings| {
                settings.orchestration_mode =
                    tycode_core::settings::config::OrchestrationMode::Builder;
            })
            .await;
        fixture.set_mock_behavior(MockBehavior::Success);

        fixture.step("how does the parser work?").await;

        let request = fixture.get_last_ai_request().expect("request captured");
        assert!(
            request
                .system_prompt
                .contains("Orchestration Policy: builder (required)"),
            "tycode must carry the required-builder policy"
        );
        let spawn_tool = request
            .tools
            .iter()
            .find(|tool| tool.name == "spawn_agent")
            .expect("tycode has spawn_agent");
        assert!(!spawn_tool.description.contains("swarm"));
        assert!(spawn_tool.description.contains("builder"));
    });
}
