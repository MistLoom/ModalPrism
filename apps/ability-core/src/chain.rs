//! 切面织入。契约在 ability-support，本模块只跑链。

use std::sync::Arc;

use ability_support::{Ability, AbilityCtx, Aspect, BoxInvoke, MethodId, Next, Result};
use serde_json::Value;

pub async fn run_chain(
    aspects: &[Arc<dyn Aspect>],
    ctx: &dyn AbilityCtx,
    ability: Arc<dyn Ability>,
    method: MethodId,
    args: Value,
) -> Result<Value> {
    let mut ordered = aspects.to_vec();
    ordered.sort_by_key(|a| a.order());
    invoke_at(0, &ordered, ctx, ability, method, args).await
}

fn invoke_at<'a>(
    index: usize,
    aspects: &'a [Arc<dyn Aspect>],
    ctx: &'a dyn AbilityCtx,
    ability: Arc<dyn Ability>,
    method: MethodId,
    args: Value,
) -> BoxInvoke<'a> {
    Box::pin(async move {
        if index < aspects.len() {
            let rest_ability = Arc::clone(&ability);
            let rest_method = method.clone();
            let aspect = Arc::clone(&aspects[index]);
            let next = Next::new(args.clone(), move |effective_args| {
                invoke_at(
                    index + 1,
                    aspects,
                    ctx,
                    rest_ability,
                    rest_method,
                    effective_args,
                )
            });
            aspect.around(ctx, &args, next).await
        } else {
            ability.invoke(&method, ctx, args).await
        }
    })
}
