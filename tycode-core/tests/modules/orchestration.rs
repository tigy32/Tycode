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
        .filter(|event| matches!(event.payload, OrchestrationPayload::AgentStarted { .. }))
        .map(|event| event.agent_type.as_str())
        .collect()
}

fn completed_agent_types(events: &[OrchestrationEvent]) -> Vec<&str> {
    events
        .iter()
        .filter(|event| matches!(event.payload, OrchestrationPayload::AgentCompleted { .. }))
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
            vec!["planner", "coder", "review"],
            "every workflow spawn must emit AgentStarted"
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
        for event in &structured {
            if let OrchestrationPayload::AgentStarted {
                parent_agent_id,
                origin,
                depth,
                interactive,
                ..
            } = &event.payload
            {
                assert!(parent_agent_id.is_some(), "workflow spawns have a parent");
                assert!(matches!(origin, AgentOrigin::Workflow));
                assert_eq!(*depth, 2, "builder children sit directly on the root");
                assert!(interactive);
            }
        }
        assert!(
            structured.iter().any(|event| matches!(
                event.payload,
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
                } => Some((*fanout_id, *total, workers.clone())),
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

        let started: Vec<u64> = structured
            .iter()
            .filter_map(|event| match &event.payload {
                OrchestrationPayload::WorkerStarted {
                    fanout_id: id,
                    worker_id,
                    ..
                } if *id == fanout_id => Some(*worker_id),
                _ => None,
            })
            .collect();
        let completed: Vec<(u64, OutcomeStatus)> = structured
            .iter()
            .filter_map(|event| match &event.payload {
                OrchestrationPayload::WorkerCompleted {
                    fanout_id: id,
                    worker_id,
                    status,
                    ..
                } if *id == fanout_id => Some((*worker_id, *status)),
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
            event.payload,
            OrchestrationPayload::FanOutCompleted {
                fanout_id: id,
                status: OutcomeStatus::Succeeded,
            } if id == fanout_id
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
        let started = structured
            .iter()
            .find(|event| matches!(event.payload, OrchestrationPayload::AgentStarted { .. }))
            .expect("spawn_agent must emit AgentStarted");
        assert_eq!(started.agent_type, "coder");
        let OrchestrationPayload::AgentStarted {
            parent_agent_id,
            task,
            origin,
            depth,
            interactive,
            ..
        } = &started.payload
        else {
            unreachable!();
        };
        assert!(parent_agent_id.is_some());
        assert_eq!(task, "write a file");
        assert!(
            matches!(origin, AgentOrigin::Tool { tool_call_id } if !tool_call_id.is_empty()),
            "tool spawns must carry the spawning tool_call_id"
        );
        assert_eq!(*depth, 2);
        assert!(interactive);

        let completed = structured
            .iter()
            .find(|event| matches!(event.payload, OrchestrationPayload::AgentCompleted { .. }))
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
            vec!["planner", "coder", "review"],
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
