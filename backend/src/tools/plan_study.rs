use std::sync::Arc;

use rig::completion::ToolAnnotations;
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::study_planning::StudyRequest;
use crate::tools::{
    ToolServices,
    support::{DomainToolError, object_schema},
};

#[derive(Deserialize)]
pub struct Args {
    pub request: StudyRequest,
}

pub struct PlanStudyTool {
    services: Arc<ToolServices>,
}

impl PlanStudyTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

impl Tool for PlanStudyTool {
    const NAME: &'static str = "plan_study";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;

    fn description(&self) -> String {
        "根据当前会话 active 模型规划静态潮流、QSTS、短路、谐波或暂态研究。调用前必须加载 study-planning skill；仅 status=ready 的结果包含 request-local studyPlanId，后续创建受管研究脚本必须原样传递该 ID。".into()
    }

    fn parameters(&self) -> Value {
        object_schema(json!({"request": study_request_schema()}), &["request"])
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        self.services.require_skill("study-planning").await?;
        let plan = self
            .services
            .study_planning
            .plan(
                &self.services.principal,
                self.services.session_id,
                args.request,
            )
            .await?;
        let ready = plan.ready.clone();
        let mut output = serde_json::to_value(plan)
            .map_err(|error| DomainToolError::new("serialization_error", error.to_string()))?;
        if let Some(ready) = ready {
            let id = self
                .services
                .ready_study_plans
                .lock()
                .await
                .insert(self.services.request_message_id, ready);
            output
                .as_object_mut()
                .expect("StudyPlan serializes as an object")
                .insert("studyPlanId".into(), Value::String(id));
        }
        Ok(output)
    }
}

fn study_request_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "studyType": {"const": "static_power_flow"},
                    "solverPreference": solver_schema(),
                    "requestedResults": string_array_schema()
                },
                "required": ["studyType"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "studyType": {"const": "qsts"},
                    "solverPreference": solver_schema(),
                    "mode": {"type": ["string", "null"], "enum": ["daily", "yearly", "duty", "peak_day", "load_duration", "ld1", "ld2", null]},
                    "startSeconds": nullable_number(),
                    "stepSizeSeconds": nullable_number(),
                    "number": {"type": ["integer", "null"], "minimum": 1, "maximum": 100000},
                    "profile": {"type": ["object", "null"]},
                    "requestedResults": string_array_schema()
                },
                "required": ["studyType"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "studyType": {"const": "fault"},
                    "cases": {"type": ["array", "null"], "maxItems": 64, "items": {"type": "object"}},
                    "allowIdealSources": {"type": ["boolean", "null"]},
                    "requestedResults": string_array_schema()
                },
                "required": ["studyType"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "studyType": {"const": "harmonic"},
                    "hMax": {"type": ["integer", "null"], "minimum": 1},
                    "monitorElements": string_array_schema(),
                    "requestedResults": string_array_schema()
                },
                "required": ["studyType"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "studyType": {"const": "transient"},
                    "subtype": {"type": ["string", "null"], "enum": ["motor_dynamics", "emt", "transient_stability", "inverter_control", "other", null]},
                    "motorDynamics": {"type": ["object", "null"], "properties": {
                        "durationSeconds": nullable_number(),
                        "stepSizeSeconds": nullable_number(),
                        "events": {"type": ["array", "null"], "maxItems": 64, "items": {"type": "object"}}
                    }, "additionalProperties": false},
                    "requestedResults": string_array_schema()
                },
                "required": ["studyType"],
                "additionalProperties": false
            }
        ]
    })
}

fn solver_schema() -> Value {
    json!({"type": "string", "enum": ["auto", "radial", "newton"], "default": "auto"})
}

fn string_array_schema() -> Value {
    json!({"type": "array", "items": {"type": "string", "maxLength": 256}, "maxItems": 64, "default": []})
}

fn nullable_number() -> Value {
    json!({"type": ["number", "null"]})
}
