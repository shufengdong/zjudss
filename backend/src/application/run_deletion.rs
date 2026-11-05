impl RunService {
    pub async fn preview_deletion(
        &self,
        user: &CurrentUser,
        id: i32,
    ) -> Result<RunDeletionPreview, RunError> {
        let token = super::opaque_token::generate_token();
        let token_hash = super::opaque_token::hash_token(&token);
        let now = now_ms();
        let expires = now + 5 * 60 * 1000;
        let impact = self
            .repo
            .preview_deletion(&user.id, id, token_hash, expires, now)
            .await?;
        Ok(RunDeletionPreview {
            impact_token: token,
            impact,
            expires_at: ms_to_iso(expires),
        })
    }

    pub async fn delete(
        &self,
        user: &CurrentUser,
        id: i32,
        impact_token: &str,
    ) -> Result<(), RunError> {
        if impact_token.len() != 64 {
            return Err(RunError::InvalidDeletionImpactToken);
        }
        let token_hash = super::opaque_token::hash_token(impact_token);
        self.repo
            .delete_run(&user.id, id, token_hash, now_ms())
            .await?;
        Ok(())
    }
}

