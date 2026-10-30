//! 全仓库能力声明规范校验。

use ability_support::{
    LintOptions, assert_clean_reports, discover_ability_roots, lint_workspace,
};
use frame_compose::workspace_root;

#[test]
fn all_ability_decls_pass_lint() {
    let workspace = workspace_root();
    let opts = LintOptions::workspace_strict();
    let reports = lint_workspace(&workspace, &opts).expect("lint workspace");
    let roots = discover_ability_roots(&workspace);
    assert_eq!(
        reports.len(),
        roots.len(),
        "lint 报告数应与发现的能力根目录一致"
    );
    assert_clean_reports(&reports).expect("声明校验失败");
}
