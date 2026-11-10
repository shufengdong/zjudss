//! API 查询参数的兼容解析 helper。

/// 按既有 wire 语义解析逗号分隔的列投影。
///
/// 空段被忽略，但保留非空段的原始空白、顺序和重复值；具体列合法性由读取 service 校验。
pub(super) fn parse_columns(value: Option<&str>) -> Vec<String> {
    value
        .map(|value| {
            value
                .split(',')
                .filter(|column| !column.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::parse_columns;

    #[test]
    fn parse_columns_preserves_existing_wire_semantics() {
        assert_eq!(parse_columns(None), Vec::<String>::new());
        assert_eq!(parse_columns(Some("a,, b,a,")), vec!["a", " b", "a"]);
    }
}
