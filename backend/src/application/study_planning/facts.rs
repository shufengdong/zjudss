use std::collections::{BTreeMap, BTreeSet, HashMap};

use arrow_array::{
    Array, ArrayRef, BooleanArray, FixedSizeListArray, LargeListArray, LargeStringArray, ListArray,
    RecordBatch, StringArray, StringViewArray,
};

use crate::application::dataframe_reader::{
    DataFrameColumn, DataFrameReadError, DataFrameReadSource, ParquetDataFrameReader,
};
use crate::application::{ArtifactService, study_planning::StudyPlanningError};
use crate::auth::CurrentUser;
use crate::db::repositories::{VersionDataframeRow, VersionSnapshot};

use super::ModelStudyFacts;

#[derive(Default)]
struct FactAccumulator {
    buses: BTreeMap<String, BTreeSet<String>>,
    edges: Vec<(String, String)>,
    elements: BTreeSet<String>,
    spectrum_names: BTreeSet<String>,
    spectrum_references: BTreeSet<String>,
    voltage_source_count: usize,
    generator_count: usize,
    ibr_count: usize,
    motor_count: usize,
    storage_count: usize,
    fault_source_equivalent_count: usize,
    has_pv_generator: bool,
    has_gfm: bool,
}

pub(super) fn empty_model_facts(engine_snapshot_id: &str) -> ModelStudyFacts {
    ModelStudyFacts {
        active_version_id: None,
        content_hash: None,
        engine_snapshot_id: engine_snapshot_id.to_string(),
        bus_count: 0,
        voltage_source_count: 0,
        generator_count: 0,
        ibr_count: 0,
        motor_count: 0,
        storage_count: 0,
        spectrum_count: 0,
        referenced_spectrum_count: 0,
        fault_source_equivalent_count: 0,
        is_connected: false,
        has_loops: false,
        has_parallel_branches: false,
        has_pv_generator: false,
        has_gfm: false,
        buses: BTreeMap::new(),
        elements: BTreeSet::new(),
    }
}

pub(super) async fn extract_active_model_facts(
    user: &CurrentUser,
    active: &VersionSnapshot,
    artifacts: &ArtifactService,
    engine_snapshot_id: &str,
) -> Result<ModelStudyFacts, StudyPlanningError> {
    let tables = active
        .dataframes
        .iter()
        .map(|row| (row.table_name.as_str(), row))
        .collect::<BTreeMap<_, _>>();
    let reader = ParquetDataFrameReader::new(artifacts.clone());
    let mut facts = FactAccumulator::default();

    facts = scan_buses(&reader, user, tables.get("bus").copied(), facts).await?;
    facts = scan_two_terminal_edges(
        &reader,
        user,
        tables.get("line").copied(),
        "line",
        &["open_from", "open_to"],
        facts,
    )
    .await?;
    facts = scan_two_terminal_edges(
        &reader,
        user,
        tables.get("switch").copied(),
        "switch",
        &["open"],
        facts,
    )
    .await?;
    facts = scan_transformers(&reader, user, tables.get("transformer").copied(), facts).await?;
    facts = scan_transformer_windings(
        &reader,
        user,
        tables.get("transformer_winding").copied(),
        facts,
    )
    .await?;
    facts = scan_sources(&reader, user, tables.get("voltage_source").copied(), facts).await?;
    facts = scan_devices(
        &reader,
        user,
        tables.get("generator").copied(),
        DeviceKind::Generator,
        facts,
    )
    .await?;
    facts = scan_devices(
        &reader,
        user,
        tables.get("ibr").copied(),
        DeviceKind::Ibr,
        facts,
    )
    .await?;
    facts = scan_devices(
        &reader,
        user,
        tables.get("motor").copied(),
        DeviceKind::Motor,
        facts,
    )
    .await?;
    facts = scan_devices(
        &reader,
        user,
        tables.get("storage").copied(),
        DeviceKind::Storage,
        facts,
    )
    .await?;
    for table in ["load", "current_source"] {
        facts = scan_spectrum_references(&reader, user, tables.get(table).copied(), table, facts)
            .await?;
    }
    facts = scan_spectra(&reader, user, tables.get("spectrum").copied(), facts).await?;

    let topology = analyze_topology(&facts.buses, &facts.edges)?;
    let referenced_spectrum_count = facts
        .spectrum_references
        .intersection(&facts.spectrum_names)
        .count();
    Ok(ModelStudyFacts {
        active_version_id: Some(active.version.id),
        content_hash: Some(active.version.content_hash.clone()),
        engine_snapshot_id: engine_snapshot_id.to_string(),
        bus_count: facts.buses.len(),
        voltage_source_count: facts.voltage_source_count,
        generator_count: facts.generator_count,
        ibr_count: facts.ibr_count,
        motor_count: facts.motor_count,
        storage_count: facts.storage_count,
        spectrum_count: facts.spectrum_names.len(),
        referenced_spectrum_count,
        fault_source_equivalent_count: facts.fault_source_equivalent_count,
        is_connected: topology.is_connected,
        has_loops: topology.has_loops,
        has_parallel_branches: topology.has_parallel_branches,
        has_pv_generator: facts.has_pv_generator,
        has_gfm: facts.has_gfm,
        buses: facts.buses,
        elements: facts.elements,
    })
}

async fn scan_buses(
    reader: &ParquetDataFrameReader,
    user: &CurrentUser,
    table: Option<&VersionDataframeRow>,
    facts: FactAccumulator,
) -> Result<FactAccumulator, StudyPlanningError> {
    let Some(table) = table else { return Ok(facts) };
    scan_table(
        reader,
        user,
        table,
        &["id"],
        &["terminals"],
        facts,
        |batch, facts| {
            for row in 0..batch.num_rows() {
                let id = required_string(batch, "id", row)?;
                let terminals = if batch.schema().index_of("terminals").is_ok() {
                    string_values(batch, "terminals", row)?
                        .into_iter()
                        .collect()
                } else {
                    BTreeSet::new()
                };
                if facts.buses.insert(id, terminals).is_some() {
                    return Err(read_error("bus.id 重复"));
                }
            }
            Ok(())
        },
    )
    .await
}

async fn scan_two_terminal_edges(
    reader: &ParquetDataFrameReader,
    user: &CurrentUser,
    table: Option<&VersionDataframeRow>,
    table_name: &'static str,
    open_columns: &'static [&'static str],
    facts: FactAccumulator,
) -> Result<FactAccumulator, StudyPlanningError> {
    let Some(table) = table else { return Ok(facts) };
    let mut optional = vec!["name", "id"];
    optional.extend_from_slice(open_columns);
    scan_table(
        reader,
        user,
        table,
        &["bus_from", "bus_to"],
        &optional,
        facts,
        move |batch, facts| {
            for row in 0..batch.num_rows() {
                let mut is_open = false;
                for column in open_columns {
                    if batch.schema().index_of(column).is_ok() && fully_open(batch, column, row)? {
                        is_open = true;
                        break;
                    }
                }
                if is_open {
                    continue;
                }
                let from = required_string(batch, "bus_from", row)?;
                let to = required_string(batch, "bus_to", row)?;
                facts.edges.push((from, to));
                if let Some(name) = element_name(batch, row)? {
                    facts.elements.insert(format!("{table_name}.{name}"));
                }
            }
            Ok(())
        },
    )
    .await
}

async fn scan_transformers(
    reader: &ParquetDataFrameReader,
    user: &CurrentUser,
    table: Option<&VersionDataframeRow>,
    facts: FactAccumulator,
) -> Result<FactAccumulator, StudyPlanningError> {
    let Some(table) = table else { return Ok(facts) };
    let available = available_columns(table)?;
    let has_from = available.contains("bus_from");
    let has_to = available.contains("bus_to");
    if has_from != has_to {
        return Err(model_error("transformer 的 bus_from/bus_to 必须成对存在"));
    }
    scan_table(
        reader,
        user,
        table,
        &["name", "subtype"],
        &["id", "bus_from", "bus_to"],
        facts,
        move |batch, facts| {
            for row in 0..batch.num_rows() {
                let subtype = required_string(batch, "subtype", row)?;
                if has_from && !subtype.eq_ignore_ascii_case("multi_winding") {
                    facts.edges.push((
                        required_string(batch, "bus_from", row)?,
                        required_string(batch, "bus_to", row)?,
                    ));
                }
                if let Some(name) = element_name(batch, row)? {
                    facts.elements.insert(format!("transformer.{name}"));
                }
            }
            Ok(())
        },
    )
    .await
}

async fn scan_transformer_windings(
    reader: &ParquetDataFrameReader,
    user: &CurrentUser,
    table: Option<&VersionDataframeRow>,
    facts: FactAccumulator,
) -> Result<FactAccumulator, StudyPlanningError> {
    let Some(table) = table else { return Ok(facts) };
    let groups = scan_table(
        reader,
        user,
        table,
        &["transformer", "bus"],
        &[],
        BTreeMap::<String, Vec<String>>::new(),
        |batch, groups| {
            for row in 0..batch.num_rows() {
                groups
                    .entry(required_string(batch, "transformer", row)?)
                    .or_default()
                    .push(required_string(batch, "bus", row)?);
            }
            Ok(())
        },
    )
    .await?;
    let mut facts = facts;
    for (name, buses) in groups {
        facts.elements.insert(format!("transformer.{name}"));
        let Some(first) = buses.first().cloned() else {
            continue;
        };
        let mut unique = BTreeSet::new();
        unique.insert(first.clone());
        for bus in buses.into_iter().skip(1) {
            if unique.insert(bus.clone()) {
                facts.edges.push((first.clone(), bus));
            }
        }
    }
    Ok(facts)
}

async fn scan_sources(
    reader: &ParquetDataFrameReader,
    user: &CurrentUser,
    table: Option<&VersionDataframeRow>,
    facts: FactAccumulator,
) -> Result<FactAccumulator, StudyPlanningError> {
    let Some(table) = table else { return Ok(facts) };
    scan_table(
        reader,
        user,
        table,
        &["name"],
        &[
            "bus", "spectrum", "sc_va", "x_r", "z1_r", "z1_x", "z2_r", "z2_x", "z0_r", "z0_x",
        ],
        facts,
        |batch, facts| {
            for row in 0..batch.num_rows() {
                facts.voltage_source_count += 1;
                let name = required_string(batch, "name", row)?;
                facts.elements.insert(format!("voltage_source.{name}"));
                capture_spectrum(batch, row, facts)?;
                if complete_group(batch, row, &["sc_va", "x_r"])
                    || complete_group(batch, row, &["z1_r", "z1_x", "z0_r", "z0_x"])
                    || complete_group(
                        batch,
                        row,
                        &["z1_r", "z1_x", "z2_r", "z2_x", "z0_r", "z0_x"],
                    )
                {
                    facts.fault_source_equivalent_count += 1;
                }
            }
            Ok(())
        },
    )
    .await
}

#[derive(Clone, Copy)]
enum DeviceKind {
    Generator,
    Ibr,
    Motor,
    Storage,
}

async fn scan_devices(
    reader: &ParquetDataFrameReader,
    user: &CurrentUser,
    table: Option<&VersionDataframeRow>,
    kind: DeviceKind,
    facts: FactAccumulator,
) -> Result<FactAccumulator, StudyPlanningError> {
    let Some(table) = table else { return Ok(facts) };
    scan_table(
        reader,
        user,
        table,
        &["name"],
        &[
            "bus",
            "execution_mode",
            "port_model",
            "spectrum",
            "z_r",
            "z_x",
            "z1_r",
            "z1_x",
            "z0_r",
            "z0_x",
            "xdpp",
            "s_rating",
            "v_nom",
        ],
        facts,
        move |batch, facts| {
            for row in 0..batch.num_rows() {
                let name = required_string(batch, "name", row)?;
                let prefix = match kind {
                    DeviceKind::Generator => {
                        facts.generator_count += 1;
                        if optional_string(batch, "execution_mode", row)?
                            .is_some_and(|value| value.eq_ignore_ascii_case("CONSTANT_PV"))
                        {
                            facts.has_pv_generator = true;
                        }
                        if complete_group(batch, row, &["z_r", "z_x"])
                            || complete_group(batch, row, &["z1_r", "z1_x", "z0_r", "z0_x"])
                            || complete_group(batch, row, &["xdpp", "s_rating", "v_nom"])
                        {
                            facts.fault_source_equivalent_count += 1;
                        }
                        "generator"
                    }
                    DeviceKind::Ibr => {
                        facts.ibr_count += 1;
                        if optional_string(batch, "port_model", row)?
                            .is_some_and(|value| value.eq_ignore_ascii_case("GFM"))
                        {
                            facts.has_gfm = true;
                        }
                        "ibr"
                    }
                    DeviceKind::Motor => {
                        facts.motor_count += 1;
                        "motor"
                    }
                    DeviceKind::Storage => {
                        facts.storage_count += 1;
                        "storage"
                    }
                };
                facts.elements.insert(format!("{prefix}.{name}"));
                capture_spectrum(batch, row, facts)?;
            }
            Ok(())
        },
    )
    .await
}

async fn scan_spectrum_references(
    reader: &ParquetDataFrameReader,
    user: &CurrentUser,
    table: Option<&VersionDataframeRow>,
    table_name: &'static str,
    facts: FactAccumulator,
) -> Result<FactAccumulator, StudyPlanningError> {
    let Some(table) = table else { return Ok(facts) };
    scan_table(
        reader,
        user,
        table,
        &["name"],
        &["id", "spectrum"],
        facts,
        move |batch, facts| {
            for row in 0..batch.num_rows() {
                if let Some(name) = element_name(batch, row)? {
                    facts.elements.insert(format!("{table_name}.{name}"));
                }
                capture_spectrum(batch, row, facts)?;
            }
            Ok(())
        },
    )
    .await
}

async fn scan_spectra(
    reader: &ParquetDataFrameReader,
    user: &CurrentUser,
    table: Option<&VersionDataframeRow>,
    facts: FactAccumulator,
) -> Result<FactAccumulator, StudyPlanningError> {
    let Some(table) = table else { return Ok(facts) };
    scan_table(
        reader,
        user,
        table,
        &["name"],
        &[],
        facts,
        |batch, facts| {
            for row in 0..batch.num_rows() {
                facts
                    .spectrum_names
                    .insert(required_string(batch, "name", row)?);
            }
            Ok(())
        },
    )
    .await
}

async fn scan_table<T, F>(
    reader: &ParquetDataFrameReader,
    user: &CurrentUser,
    table: &VersionDataframeRow,
    required: &[&str],
    optional: &[&str],
    state: T,
    visit: F,
) -> Result<T, StudyPlanningError>
where
    T: Send + 'static,
    F: FnMut(&RecordBatch, &mut T) -> Result<(), DataFrameReadError> + Send + 'static,
{
    let available = available_columns(table)?;
    for column in required {
        if !available.contains(*column) {
            return Err(model_error(format!(
                "{} schema 缺少必需列 {column}",
                table.table_name
            )));
        }
    }
    let columns = required
        .iter()
        .chain(optional.iter())
        .filter(|column| available.contains(**column))
        .map(|column| (*column).to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    reader
        .scan_projected(user, &source(table), &columns, state, visit)
        .await
        .map_err(|error| model_error(format!("{}: {error}", table.table_name)))
}

fn available_columns(table: &VersionDataframeRow) -> Result<BTreeSet<String>, StudyPlanningError> {
    serde_json::from_str::<Vec<DataFrameColumn>>(&table.schema_json)
        .map(|columns| columns.into_iter().map(|column| column.name).collect())
        .map_err(|error| model_error(format!("{} schema 无效: {error}", table.table_name)))
}

fn source(table: &VersionDataframeRow) -> DataFrameReadSource {
    DataFrameReadSource {
        resource_key: format!("model-dataframe:{}", table.id),
        artifact_id: table.artifact_id,
        schema_json: table.schema_json.clone(),
        row_count: table.row_count,
        content_hash: table.content_hash.clone(),
    }
}

fn required_string(
    batch: &RecordBatch,
    column: &str,
    row: usize,
) -> Result<String, DataFrameReadError> {
    optional_string(batch, column, row)?.ok_or_else(|| read_error(format!("{column} 不能为空")))
}

fn optional_string(
    batch: &RecordBatch,
    column: &str,
    row: usize,
) -> Result<Option<String>, DataFrameReadError> {
    let Ok(index) = batch.schema().index_of(column) else {
        return Ok(None);
    };
    let array = batch.column(index);
    if array.is_null(row) {
        return Ok(None);
    }
    string_from_array(array, row).map(Some)
}

fn string_from_array(array: &ArrayRef, row: usize) -> Result<String, DataFrameReadError> {
    if let Some(array) = array.as_any().downcast_ref::<StringArray>() {
        return Ok(array.value(row).to_string());
    }
    if let Some(array) = array.as_any().downcast_ref::<LargeStringArray>() {
        return Ok(array.value(row).to_string());
    }
    if let Some(array) = array.as_any().downcast_ref::<StringViewArray>() {
        return Ok(array.value(row).to_string());
    }
    Err(read_error("字符串列类型不受支持"))
}

fn string_values(
    batch: &RecordBatch,
    column: &str,
    row: usize,
) -> Result<Vec<String>, DataFrameReadError> {
    let index = batch
        .schema()
        .index_of(column)
        .map_err(|_| read_error(format!("缺少列 {column}")))?;
    let array = batch.column(index);
    if array.is_null(row) {
        return Ok(vec![]);
    }
    if array.as_any().is::<StringArray>()
        || array.as_any().is::<LargeStringArray>()
        || array.as_any().is::<StringViewArray>()
    {
        return string_from_array(array, row).map(|value| vec![value]);
    }
    let values = if let Some(array) = array.as_any().downcast_ref::<ListArray>() {
        array.value(row)
    } else if let Some(array) = array.as_any().downcast_ref::<LargeListArray>() {
        array.value(row)
    } else if let Some(array) = array.as_any().downcast_ref::<FixedSizeListArray>() {
        array.value(row)
    } else {
        return Err(read_error("字符串列表列类型不受支持"));
    };
    (0..values.len())
        .filter(|index| !values.is_null(*index))
        .map(|index| string_from_array(&values, index))
        .collect()
}

fn fully_open(batch: &RecordBatch, column: &str, row: usize) -> Result<bool, DataFrameReadError> {
    let index = batch
        .schema()
        .index_of(column)
        .map_err(|_| read_error(format!("缺少列 {column}")))?;
    let array = batch.column(index);
    if array.is_null(row) {
        return Ok(false);
    }
    if let Some(array) = array.as_any().downcast_ref::<BooleanArray>() {
        return Ok(array.value(row));
    }
    if array.as_any().is::<StringArray>()
        || array.as_any().is::<LargeStringArray>()
        || array.as_any().is::<StringViewArray>()
    {
        return Ok(string_from_array(array, row)?.eq_ignore_ascii_case("true"));
    }
    let values = if let Some(array) = array.as_any().downcast_ref::<ListArray>() {
        array.value(row)
    } else if let Some(array) = array.as_any().downcast_ref::<LargeListArray>() {
        array.value(row)
    } else if let Some(array) = array.as_any().downcast_ref::<FixedSizeListArray>() {
        array.value(row)
    } else {
        return Err(read_error("开关状态列类型不受支持"));
    };
    let values = values
        .as_any()
        .downcast_ref::<BooleanArray>()
        .ok_or_else(|| read_error("开关状态列表必须是布尔值"))?;
    Ok(!values.is_empty()
        && (0..values.len()).all(|index| !values.is_null(index) && values.value(index)))
}

fn element_name(batch: &RecordBatch, row: usize) -> Result<Option<String>, DataFrameReadError> {
    optional_string(batch, "name", row).and_then(|name| {
        if name.is_some() {
            Ok(name)
        } else {
            optional_string(batch, "id", row)
        }
    })
}

fn capture_spectrum(
    batch: &RecordBatch,
    row: usize,
    facts: &mut FactAccumulator,
) -> Result<(), DataFrameReadError> {
    if let Some(spectrum) = optional_string(batch, "spectrum", row)?
        && !spectrum.is_empty()
    {
        facts.spectrum_references.insert(spectrum);
    }
    Ok(())
}

fn complete_group(batch: &RecordBatch, row: usize, columns: &[&str]) -> bool {
    columns.iter().all(|column| {
        batch
            .schema()
            .index_of(column)
            .ok()
            .is_some_and(|index| !batch.column(index).is_null(row))
    })
}

struct TopologyFacts {
    is_connected: bool,
    has_loops: bool,
    has_parallel_branches: bool,
}

fn analyze_topology(
    buses: &BTreeMap<String, BTreeSet<String>>,
    edges: &[(String, String)],
) -> Result<TopologyFacts, StudyPlanningError> {
    if buses.is_empty() {
        return Ok(TopologyFacts {
            is_connected: false,
            has_loops: false,
            has_parallel_branches: false,
        });
    }
    let indices = buses
        .keys()
        .enumerate()
        .map(|(index, name)| (name.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut union = UnionFind::new(buses.len());
    let mut pairs = BTreeMap::<(usize, usize), usize>::new();
    let mut has_loops = false;
    for (from, to) in edges {
        let from_index = indices
            .get(from.as_str())
            .copied()
            .ok_or_else(|| model_error(format!("支路引用不存在的母线 {from}")))?;
        let to_index = indices
            .get(to.as_str())
            .copied()
            .ok_or_else(|| model_error(format!("支路引用不存在的母线 {to}")))?;
        let pair = if from_index <= to_index {
            (from_index, to_index)
        } else {
            (to_index, from_index)
        };
        *pairs.entry(pair).or_default() += 1;
        if !union.join(from_index, to_index) {
            has_loops = true;
        }
    }
    let root = union.find(0);
    let is_connected = (1..buses.len()).all(|index| union.find(index) == root);
    Ok(TopologyFacts {
        is_connected,
        has_loops,
        has_parallel_branches: pairs.values().any(|count| *count > 1),
    })
}

struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl UnionFind {
    fn new(size: usize) -> Self {
        Self {
            parent: (0..size).collect(),
            rank: vec![0; size],
        }
    }

    fn find(&mut self, value: usize) -> usize {
        if self.parent[value] != value {
            self.parent[value] = self.find(self.parent[value]);
        }
        self.parent[value]
    }

    fn join(&mut self, left: usize, right: usize) -> bool {
        let mut left = self.find(left);
        let mut right = self.find(right);
        if left == right {
            return false;
        }
        if self.rank[left] < self.rank[right] {
            std::mem::swap(&mut left, &mut right);
        }
        self.parent[right] = left;
        if self.rank[left] == self.rank[right] {
            self.rank[left] += 1;
        }
        true
    }
}

fn read_error(message: impl Into<String>) -> DataFrameReadError {
    DataFrameReadError::Storage(message.into())
}

fn model_error(message: impl Into<String>) -> StudyPlanningError {
    StudyPlanningError::ModelReadFailed(message.into())
}
