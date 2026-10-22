//! 仅把调用链范围交给支撑层反馈；不实现记录。

use ability_support::FeedbackScope;

tokio::task_local! {
    static SCOPE: FeedbackScope;
}

pub fn current_task_scope() -> FeedbackScope {
    SCOPE
        .try_with(|s| s.clone())
        .unwrap_or_else(|_| FeedbackScope::detached())
}

pub async fn with_scope<F, T>(scope: FeedbackScope, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    SCOPE.scope(scope, fut).await
}

pub fn bind_scope() {
    ability_support::feedback::host::install_scope(current_task_scope);
}
