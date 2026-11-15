use crate::auth::UserId;
use crate::db::repositories::{DatasetModelOrigin, ResultRepo};

use super::AnalysisError;

pub(super) async fn require_matching_dataset_origins(
    results: &ResultRepo,
    user: &UserId,
    session_id: i32,
    dataset_ids: &[i32],
    active_version_id: i32,
) -> Result<(), AnalysisError> {
    for dataset_id in dataset_ids {
        let origin = results
            .dataset_model_origin(user, session_id, *dataset_id, 64)
            .await
            .map_err(|_| AnalysisError::new("analysis_input_not_found", "资源不存在"))?;
        if origin != DatasetModelOrigin::Unique(active_version_id) {
            return Err(AnalysisError::new(
                "analysis_model_origin_mismatch",
                format!("数据集 {dataset_id} 不属于当前激活模型版本"),
            ));
        }
    }
    Ok(())
}
