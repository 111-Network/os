//! /health model. 200 for ok|degraded (the process is alive — a dead hub or
//! absent KB must not make 111d restart-loop us); 503 only for error
//! (state_dir unusable, i.e. the agent itself is broken).

use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub struct Components {
    pub model: bool,
    pub kb: bool,
    pub state_dir: bool,
}

#[derive(Debug, Serialize)]
pub struct ComponentsBody {
    pub model: bool,
    pub kb: bool,
    pub state_dir: bool,
}

#[derive(Debug, Serialize)]
pub struct HealthBody {
    pub schema: u8,
    pub status: &'static str,
    pub components: ComponentsBody,
}

pub fn evaluate(c: &Components) -> (axum::http::StatusCode, HealthBody) {
    use axum::http::StatusCode;
    let (code, status) = if !c.state_dir {
        (StatusCode::SERVICE_UNAVAILABLE, "error")
    } else if !c.model || !c.kb {
        (StatusCode::OK, "degraded")
    } else {
        (StatusCode::OK, "ok")
    };
    (
        code,
        HealthBody {
            schema: 1,
            status,
            components: ComponentsBody {
                model: c.model,
                kb: c.kb,
                state_dir: c.state_dir,
            },
        },
    )
}
