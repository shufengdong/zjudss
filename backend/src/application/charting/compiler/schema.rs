use super::*;

pub(super) fn line_plot_schema() -> Vec<DataFrameColumn> {
    vec![
        plot_column("ordinal", "UInt64", false),
        plot_column("x_kind", "Utf8", false),
        plot_column("x_category_json", "Utf8", true),
        plot_column("x_value", "Float64", true),
        plot_column("series", "Utf8", false),
        plot_column("value", "Float64", false),
    ]
}

pub(super) fn bar_plot_schema() -> Vec<DataFrameColumn> {
    vec![
        plot_column("category_index", "UInt64", false),
        plot_column("category_json", "Utf8", false),
        plot_column("series", "Utf8", false),
        plot_column("value", "Float64", true),
    ]
}

pub(super) fn scatter_plot_schema() -> Vec<DataFrameColumn> {
    vec![
        plot_column("ordinal", "UInt64", false),
        plot_column("series", "Utf8", false),
        plot_column("x", "Float64", false),
        plot_column("y", "Float64", false),
    ]
}

pub(super) fn heatmap_plot_schema() -> Vec<DataFrameColumn> {
    vec![
        plot_column("x_index", "UInt64", false),
        plot_column("x_category_json", "Utf8", false),
        plot_column("y_index", "UInt64", false),
        plot_column("y_category_json", "Utf8", false),
        plot_column("value", "Float64", false),
    ]
}

pub(super) fn boxplot_plot_schema() -> Vec<DataFrameColumn> {
    vec![
        plot_column("category_index", "UInt64", false),
        plot_column("category_json", "Utf8", false),
        plot_column("series", "Utf8", false),
        plot_column("min", "Float64", false),
        plot_column("q1", "Float64", false),
        plot_column("median", "Float64", false),
        plot_column("q3", "Float64", false),
        plot_column("max", "Float64", false),
        plot_column("outliers", "Utf8", false),
    ]
}

pub(super) fn histogram_plot_schema() -> Vec<DataFrameColumn> {
    vec![
        plot_column("bin_index", "UInt64", false),
        plot_column("bin_start", "Float64", false),
        plot_column("bin_end", "Float64", false),
        plot_column("count", "UInt64", false),
    ]
}

pub(super) fn plot_column(name: &str, dtype: &str, nullable: bool) -> DataFrameColumn {
    DataFrameColumn {
        name: name.into(),
        dtype: dtype.into(),
        nullable,
        unit: None,
    }
}
