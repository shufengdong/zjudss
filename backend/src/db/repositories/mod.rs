mod message;
mod session;
mod task;

pub use message::MessageRepo;
pub use session::SessionRepo;
pub use task::TaskRepo;

#[derive(Debug)]
pub enum RepoError {
    Pool(diesel::r2d2::PoolError),
    Query(diesel::result::Error),
    Join(tokio::task::JoinError),
    NotFound,
}

impl std::fmt::Display for RepoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoError::Pool(error) => write!(f, "连接池错误: {error}"),
            RepoError::Query(error) => write!(f, "查询错误: {error}"),
            RepoError::Join(error) => write!(f, "阻塞任务错误: {error}"),
            RepoError::NotFound => write!(f, "资源不存在"),
        }
    }
}

impl std::error::Error for RepoError {}

pub type RepoResult<T> = Result<T, RepoError>;
