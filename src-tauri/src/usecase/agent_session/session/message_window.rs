use serde::{Deserialize, Serialize};

use crate::usecase::agent_session::status::TurnPhase;

pub const RETAINED_MESSAGE_CAP: usize = 200;
pub const MAX_HYDRATED_SESSIONS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedMessagePage {
    pub request_cursor: Option<String>,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveMessageWindowObservation {
    pub session_id: String,
    pub message_count: usize,
    pub oldest_visible_index: usize,
    #[serde(default)]
    pub loaded_pages: Vec<LoadedMessagePage>,
    pub turn_phase: TurnPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HydratedSessionObservation {
    pub session_id: String,
    pub message_count: usize,
    pub eviction_rank: u64,
    pub protected: bool,
    pub loading: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChatEvictionPlanRequest {
    #[serde(default)]
    pub active: Option<ActiveMessageWindowObservation>,
    #[serde(default)]
    pub sessions: Vec<HydratedSessionObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageEvictionDirection {
    Older,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveMessageEvictionPlan {
    pub session_id: String,
    pub direction: MessageEvictionDirection,
    pub count: usize,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub loaded_pages: Vec<LoadedMessagePage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChatEvictionPlan {
    pub active: Option<ActiveMessageEvictionPlan>,
    pub evict_session_ids: Vec<String>,
}

pub fn plan_agent_chat_eviction(request: AgentChatEvictionPlanRequest) -> AgentChatEvictionPlan {
    AgentChatEvictionPlan {
        active: request
            .active
            .as_ref()
            .and_then(plan_active_window_eviction),
        evict_session_ids: plan_inactive_session_eviction(&request.sessions),
    }
}

fn plan_inactive_session_eviction(sessions: &[HydratedSessionObservation]) -> Vec<String> {
    let hydrated_count = sessions
        .iter()
        .filter(|session| session.message_count > 0)
        .count();
    let excess = hydrated_count.saturating_sub(MAX_HYDRATED_SESSIONS);
    if excess == 0 {
        return Vec::new();
    }

    let mut candidates = sessions
        .iter()
        .filter(|session| session.message_count > 0 && !session.protected && !session.loading)
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.eviction_rank
            .cmp(&right.eviction_rank)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });

    candidates
        .into_iter()
        .take(excess)
        .map(|session| session.session_id.clone())
        .collect()
}

fn plan_active_window_eviction(
    observation: &ActiveMessageWindowObservation,
) -> Option<ActiveMessageEvictionPlan> {
    if observation.turn_phase != TurnPhase::Idle {
        return None;
    }
    if observation.message_count <= RETAINED_MESSAGE_CAP {
        return None;
    }

    let loaded_pages = sanitize_pages(observation.loaded_pages.clone());
    if loaded_pages.len() <= 1 {
        return None;
    }

    plan_older_eviction(observation, &loaded_pages)
}

fn plan_older_eviction(
    observation: &ActiveMessageWindowObservation,
    loaded_pages: &[LoadedMessagePage],
) -> Option<ActiveMessageEvictionPlan> {
    debug_assert!(observation.message_count > RETAINED_MESSAGE_CAP);
    debug_assert!(loaded_pages.len() > 1);

    let max_drop_count = observation
        .oldest_visible_index
        .min(observation.message_count);
    if max_drop_count == 0 {
        return None;
    }

    let mut retained_pages = loaded_pages.to_vec();
    let mut dropped_pages = Vec::new();
    let mut retained_message_count = observation.message_count;
    let mut drop_count = 0usize;

    while retained_message_count > RETAINED_MESSAGE_CAP && retained_pages.len() > 1 {
        let page = retained_pages.last().cloned()?;
        if drop_count + page.count > max_drop_count {
            break;
        }
        retained_pages.pop();
        drop_count += page.count;
        retained_message_count = retained_message_count.saturating_sub(page.count);
        dropped_pages.push(page);
    }

    if drop_count == 0 {
        return None;
    }

    let rewind_page = dropped_pages.last()?;
    Some(ActiveMessageEvictionPlan {
        session_id: observation.session_id.clone(),
        direction: MessageEvictionDirection::Older,
        count: drop_count,
        next_cursor: rewind_page.request_cursor.clone(),
        has_more: true,
        loaded_pages: retained_pages,
    })
}

fn sanitize_pages(pages: Vec<LoadedMessagePage>) -> Vec<LoadedMessagePage> {
    pages.into_iter().filter(|page| page.count > 0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(request_cursor: Option<&str>, count: usize) -> LoadedMessagePage {
        LoadedMessagePage {
            request_cursor: request_cursor.map(str::to_string),
            count,
        }
    }

    fn loaded_message_count(pages: &[LoadedMessagePage]) -> usize {
        pages.iter().map(|page| page.count).sum()
    }

    fn active_observation() -> ActiveMessageWindowObservation {
        ActiveMessageWindowObservation {
            session_id: "s1".to_string(),
            message_count: 250,
            oldest_visible_index: 60,
            loaded_pages: vec![
                page(None, 50),
                page(Some("201"), 50),
                page(Some("151"), 50),
                page(Some("101"), 50),
                page(Some("51"), 50),
            ],
            turn_phase: TurnPhase::Idle,
        }
    }

    #[test]
    fn active_eviction_drops_oldest_pages_when_prefix_is_outside_visible_range() {
        let plan = plan_agent_chat_eviction(AgentChatEvictionPlanRequest {
            active: Some(active_observation()),
            sessions: Vec::new(),
        })
        .active
        .expect("active plan");

        assert_eq!(plan.direction, MessageEvictionDirection::Older);
        assert_eq!(plan.count, 50);
        assert_eq!(plan.next_cursor.as_deref(), Some("51"));
        assert!(plan.has_more);
        assert_eq!(plan.loaded_pages.len(), 4);
        assert_eq!(
            loaded_message_count(&plan.loaded_pages),
            RETAINED_MESSAGE_CAP
        );
        assert_eq!(
            plan.loaded_pages.last().unwrap().request_cursor.as_deref(),
            Some("101")
        );
    }

    #[test]
    fn active_eviction_keeps_live_tail_when_user_is_scrolling_at_top() {
        let mut observation = active_observation();
        observation.oldest_visible_index = 0;

        let plan = plan_agent_chat_eviction(AgentChatEvictionPlanRequest {
            active: Some(observation),
            sessions: Vec::new(),
        });

        assert!(plan.active.is_none());
    }

    #[test]
    fn active_eviction_keeps_non_idle_window_intact() {
        for turn_phase in [TurnPhase::Streaming, TurnPhase::WaitingPermission] {
            let mut observation = active_observation();
            observation.turn_phase = turn_phase;

            let plan = plan_agent_chat_eviction(AgentChatEvictionPlanRequest {
                active: Some(observation),
                sessions: Vec::new(),
            });

            assert!(plan.active.is_none());
        }
    }

    #[test]
    fn active_eviction_returns_partial_plan_when_live_tail_exceeds_cap() {
        let mut observation = active_observation();
        observation.message_count = 350;
        observation.oldest_visible_index = 100;
        observation.loaded_pages = vec![
            page(None, 250),
            page(Some("251"), 50),
            page(Some("201"), 50),
        ];

        let plan = plan_agent_chat_eviction(AgentChatEvictionPlanRequest {
            active: Some(observation),
            sessions: Vec::new(),
        })
        .active
        .expect("partial active plan");

        assert_eq!(plan.count, 100);
        assert_eq!(plan.next_cursor.as_deref(), Some("251"));
        assert!(plan.has_more);
        assert_eq!(plan.loaded_pages, vec![page(None, 250)]);
        assert!(loaded_message_count(&plan.loaded_pages) > RETAINED_MESSAGE_CAP);
    }

    #[test]
    fn active_eviction_returns_none_at_or_below_cap() {
        for message_count in [RETAINED_MESSAGE_CAP - 1, RETAINED_MESSAGE_CAP] {
            let mut observation = active_observation();
            observation.message_count = message_count;
            observation.oldest_visible_index = 50;
            observation.loaded_pages = vec![
                page(None, 50),
                page(Some("151"), 50),
                page(Some("101"), 50),
                page(Some("51"), message_count.saturating_sub(150)),
            ];

            let plan = plan_agent_chat_eviction(AgentChatEvictionPlanRequest {
                active: Some(observation),
                sessions: Vec::new(),
            });

            assert!(plan.active.is_none());
        }
    }

    #[test]
    fn active_eviction_returns_none_for_single_live_tail_page() {
        let mut observation = active_observation();
        observation.message_count = RETAINED_MESSAGE_CAP + 50;
        observation.oldest_visible_index = 50;
        observation.loaded_pages = vec![page(None, RETAINED_MESSAGE_CAP + 50)];

        let plan = plan_agent_chat_eviction(AgentChatEvictionPlanRequest {
            active: Some(observation),
            sessions: Vec::new(),
        });

        assert!(plan.active.is_none());
    }

    #[test]
    fn inactive_eviction_returns_unprotected_hydrated_sessions_above_cap() {
        let sessions = [("s2", 20), ("s5", 5), ("s4", 1), ("s1", 10), ("s3", 0)]
            .into_iter()
            .map(|(session_id, eviction_rank)| HydratedSessionObservation {
                session_id: session_id.to_string(),
                message_count: 50,
                eviction_rank,
                protected: matches!(session_id, "s3"),
                loading: matches!(session_id, "s4"),
            })
            .collect();

        let plan = plan_agent_chat_eviction(AgentChatEvictionPlanRequest {
            active: None,
            sessions,
        });

        assert_eq!(
            plan.evict_session_ids,
            vec!["s5".to_string(), "s1".to_string()]
        );
    }
}
