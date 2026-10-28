//! 出站 API 契约表非空且可被 compat 测试引用。

use ability_plugin_host::mod_outbound_surfaces;

#[test]
fn outbound_table_nonempty() {
    assert!(mod_outbound_surfaces().len() >= 10);
}
