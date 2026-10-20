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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    fn eval(model: bool, kb: bool, state_dir: bool) -> (StatusCode, &'static str) {
        let (code, body) = evaluate(&Components {
            model,
            kb,
            state_dir,
        });
        assert_eq!(body.schema, 1);
        assert_eq!(body.components.model, model);
        assert_eq!(body.components.kb, kb);
        assert_eq!(body.components.state_dir, state_dir);
        (code, body.status)
    }

    #[test]
    fn state_dir_down_is_503_error() {
        // state_dir unusable => the agent itself is broken, whatever the rest.
        for (model, kb) in [(false, false), (true, false), (false, true), (true, true)] {
            let (code, status) = eval(model, kb, false);
            assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE, "{model}/{kb}");
            assert_eq!(status, "error", "{model}/{kb}");
        }
    }

    #[test]
    fn model_or_kb_down_is_200_degraded() {
        // A dead hub or absent KB must not restart-loop the agent: 200.
        for (model, kb) in [(false, false), (true, false), (false, true)] {
            let (code, status) = eval(model, kb, true);
            assert_eq!(code, StatusCode::OK, "{model}/{kb}");
            assert_eq!(status, "degraded", "{model}/{kb}");
        }
    }

    #[test]
    fn all_up_is_200_ok() {
        let (code, status) = eval(true, true, true);
        assert_eq!(code, StatusCode::OK);
        assert_eq!(status, "ok");
    }
}
