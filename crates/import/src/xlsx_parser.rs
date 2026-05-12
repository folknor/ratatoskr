use std::collections::HashMap;
use std::io::{Cursor, Read};

use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, Event};
use zip::read::ZipArchive;

use crate::table::{build_table_preview, prepare_table_import};
use crate::types::{
    ColumnMapping, ImportError, ImportFormat, ImportOptions, ImportSource, PreparedImport,
    SheetInfo, TablePreview,
};

#[derive(Debug, Clone)]
struct WorkbookSheet {
    info: SheetInfo,
    rel_id: String,
    path: String,
}

type XlsxRows = (Vec<Vec<String>>, Vec<SheetInfo>, usize);

const MAX_XLSX_XML_BYTES: u64 = 64 * 1024 * 1024;

/// Parse an XLSX workbook into a table preview.
pub fn preview_xlsx(
    source: &ImportSource,
    options: ImportOptions,
) -> Result<TablePreview, ImportError> {
    let (rows, sheets, selected_sheet) = load_xlsx_rows(source, options.sheet_index)?;
    build_table_preview(
        ImportFormat::Xlsx,
        rows,
        None,
        sheets,
        Some(selected_sheet),
        options,
    )
}

/// Prepare XLSX contacts using caller-provided mappings.
pub fn prepare_xlsx_import(
    source: &ImportSource,
    mappings: &[ColumnMapping],
    options: ImportOptions,
) -> Result<PreparedImport, ImportError> {
    let (rows, _, _) = load_xlsx_rows(source, options.sheet_index)?;
    let has_header = options
        .has_header
        .unwrap_or_else(|| crate::detect::detect_has_header(&rows));
    Ok(prepare_table_import(rows, mappings, has_header))
}

fn load_xlsx_rows(
    source: &ImportSource,
    sheet_index: Option<usize>,
) -> Result<XlsxRows, ImportError> {
    let mut archive = open_archive(&source.data)?;
    let shared_strings = read_optional_zip_text(&mut archive, "xl/sharedStrings.xml")?
        .map(|xml| parse_shared_strings(&xml))
        .transpose()?
        .unwrap_or_default();
    let sheets = read_workbook_sheets(&mut archive)?;
    if sheets.is_empty() {
        return Err(ImportError::ParseError("XLSX workbook has no sheets".to_string()));
    }

    let selected = sheet_index.unwrap_or(0);
    let sheet = sheets
        .get(selected)
        .ok_or_else(|| ImportError::ParseError(format!("XLSX sheet index {selected} not found")))?;
    let sheet_xml = read_zip_text(&mut archive, &sheet.path)?;
    let rows = parse_sheet_rows(&sheet_xml, &shared_strings)?;
    let sheet_infos = sheets.into_iter().map(|sheet| sheet.info).collect();

    Ok((rows, sheet_infos, selected))
}

fn open_archive(data: &[u8]) -> Result<ZipArchive<Cursor<&[u8]>>, ImportError> {
    ZipArchive::new(Cursor::new(data)).map_err(|e| ImportError::ParseError(e.to_string()))
}

fn read_workbook_sheets(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
) -> Result<Vec<WorkbookSheet>, ImportError> {
    let workbook = read_zip_text(archive, "xl/workbook.xml")?;
    let rels = read_zip_text(archive, "xl/_rels/workbook.xml.rels")?;
    let rel_targets = parse_workbook_relationships(&rels)?;
    let mut sheets = parse_workbook_sheet_list(&workbook)?;

    for sheet in &mut sheets {
        let Some(target) = rel_targets.get(&sheet.rel_id) else {
            return Err(ImportError::ParseError(format!(
                "XLSX sheet relationship {} missing",
                sheet.rel_id
            )));
        };
        sheet.path = normalize_workbook_target(target);
    }

    Ok(sheets)
}

fn read_zip_text(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    path: &str,
) -> Result<String, ImportError> {
    let mut file = archive
        .by_name(path)
        .map_err(|e| ImportError::ParseError(format!("{path}: {e}")))?;
    if file.size() > MAX_XLSX_XML_BYTES {
        return Err(ImportError::ParseError(format!(
            "{path}: XML part is too large for contact import"
        )));
    }
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| ImportError::ParseError(format!("{path}: {e}")))?;
    Ok(text)
}

fn read_optional_zip_text(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    path: &str,
) -> Result<Option<String>, ImportError> {
    match archive.by_name(path) {
        Ok(mut file) => {
            if file.size() > MAX_XLSX_XML_BYTES {
                return Err(ImportError::ParseError(format!(
                    "{path}: XML part is too large for contact import"
                )));
            }
            let mut text = String::new();
            file.read_to_string(&mut text)
                .map_err(|e| ImportError::ParseError(format!("{path}: {e}")))?;
            Ok(Some(text))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(ImportError::ParseError(format!("{path}: {e}"))),
    }
}

fn parse_workbook_sheet_list(xml: &str) -> Result<Vec<WorkbookSheet>, ImportError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text_start = true;
    reader.config_mut().trim_text_end = true;
    let mut buf = Vec::new();
    let mut sheets = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local_name(e.name().as_ref()) == b"sheet" => {
                let name = attr_value(&e, b"name").unwrap_or_else(|| {
                    let next = sheets.len() + 1;
                    format!("Sheet {next}")
                });
                let rel_id = attr_value(&e, b"r:id").unwrap_or_default();
                let index = sheets.len();
                sheets.push(WorkbookSheet {
                    info: SheetInfo { index, name },
                    rel_id,
                    path: String::new(),
                });
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(ImportError::ParseError(format!("workbook.xml: {e}"))),
        }
        buf.clear();
    }

    Ok(sheets)
}

fn parse_workbook_relationships(xml: &str) -> Result<HashMap<String, String>, ImportError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text_start = true;
    reader.config_mut().trim_text_end = true;
    let mut buf = Vec::new();
    let mut rels = HashMap::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e))
                if local_name(e.name().as_ref()) == b"Relationship" =>
            {
                if let (Some(id), Some(target)) = (attr_value(&e, b"Id"), attr_value(&e, b"Target")) {
                    rels.insert(id, target);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => {
                return Err(ImportError::ParseError(format!(
                    "workbook relationships: {e}"
                )));
            }
        }
        buf.clear();
    }

    Ok(rels)
}

fn parse_shared_strings(xml: &str) -> Result<Vec<String>, ImportError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text_start = false;
    reader.config_mut().trim_text_end = false;
    let mut buf = Vec::new();
    let mut strings = Vec::new();
    let mut in_si = false;
    let mut in_t = false;
    let mut current = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if local_name(e.name().as_ref()) == b"si" => {
                in_si = true;
                current.clear();
            }
            Ok(Event::End(e)) if local_name(e.name().as_ref()) == b"si" => {
                strings.push(current.clone());
                current.clear();
                in_si = false;
            }
            Ok(Event::Start(e)) if in_si && local_name(e.name().as_ref()) == b"t" => {
                in_t = true;
            }
            Ok(Event::End(e)) if local_name(e.name().as_ref()) == b"t" => {
                in_t = false;
            }
            Ok(Event::Text(e)) if in_si && in_t => {
                current.push_str(&xml_text(e.as_ref()));
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(ImportError::ParseError(format!("sharedStrings.xml: {e}"))),
        }
        buf.clear();
    }

    Ok(strings)
}

fn parse_sheet_rows(xml: &str, shared_strings: &[String]) -> Result<Vec<Vec<String>>, ImportError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text_start = true;
    reader.config_mut().trim_text_end = true;
    let mut buf = Vec::new();
    let mut rows = Vec::new();
    let mut current_row: Option<Vec<String>> = None;
    let mut current_cell_col = 0usize;
    let mut current_cell_type = String::new();
    let mut current_cell_text = String::new();
    let mut in_cell = false;
    let mut in_value_text = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if local_name(e.name().as_ref()) == b"row" => {
                current_row = Some(Vec::new());
            }
            Ok(Event::End(e)) if local_name(e.name().as_ref()) == b"row" => {
                if let Some(mut row) = current_row.take() {
                    trim_trailing_empty_cells(&mut row);
                    if !row.iter().all(String::is_empty) {
                        rows.push(row);
                    }
                }
            }
            Ok(Event::Start(e)) if local_name(e.name().as_ref()) == b"c" => {
                in_cell = true;
                current_cell_text.clear();
                current_cell_type = attr_value(&e, b"t").unwrap_or_default();
                current_cell_col = attr_value(&e, b"r")
                    .and_then(|r| column_index_from_cell_ref(&r))
                    .unwrap_or_else(|| {
                        current_row
                            .as_ref()
                            .map(Vec::len)
                            .unwrap_or_default()
                    });
            }
            Ok(Event::End(e)) if local_name(e.name().as_ref()) == b"c" => {
                if let Some(row) = current_row.as_mut() {
                    let value =
                        resolve_cell_value(&current_cell_text, &current_cell_type, shared_strings);
                    set_cell(row, current_cell_col, &value);
                }
                in_cell = false;
                in_value_text = false;
                current_cell_text.clear();
                current_cell_type.clear();
            }
            Ok(Event::Start(e))
                if in_cell
                    && matches!(local_name(e.name().as_ref()), b"v" | b"t") =>
            {
                in_value_text = true;
            }
            Ok(Event::End(e)) if matches!(local_name(e.name().as_ref()), b"v" | b"t") => {
                in_value_text = false;
            }
            Ok(Event::Text(e)) if in_cell && in_value_text => {
                current_cell_text.push_str(&xml_text(e.as_ref()));
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(ImportError::ParseError(format!("worksheet XML: {e}"))),
        }
        buf.clear();
    }

    Ok(rows)
}

fn resolve_cell_value(raw: &str, cell_type: &str, shared_strings: &[String]) -> String {
    let trimmed = raw.trim();
    match cell_type {
        "s" => trimmed
            .parse::<usize>()
            .ok()
            .and_then(|index| shared_strings.get(index))
            .cloned()
            .unwrap_or_default(),
        "b" => match trimmed {
            "1" => "TRUE".to_string(),
            "0" => "FALSE".to_string(),
            _ => trimmed.to_string(),
        },
        _ => trimmed.to_string(),
    }
}

fn set_cell(row: &mut Vec<String>, index: usize, value: &str) {
    if row.len() <= index {
        row.resize(index + 1, String::new());
    }
    row[index] = value.trim().to_string();
}

fn trim_trailing_empty_cells(row: &mut Vec<String>) {
    while row.last().is_some_and(String::is_empty) {
        let _ = row.pop();
    }
}

fn normalize_workbook_target(target: &str) -> String {
    if let Some(stripped) = target.strip_prefix('/') {
        stripped.to_string()
    } else if target.starts_with("xl/") {
        target.to_string()
    } else {
        format!("xl/{target}")
    }
}

fn column_index_from_cell_ref(cell_ref: &str) -> Option<usize> {
    let mut value = 0usize;
    let mut saw_letter = false;
    for byte in cell_ref.bytes() {
        if !byte.is_ascii_alphabetic() {
            break;
        }
        saw_letter = true;
        let upper = byte.to_ascii_uppercase();
        let digit = usize::from(upper - b'A' + 1);
        value = value.checked_mul(26)?.checked_add(digit)?;
    }
    saw_letter.then_some(value - 1)
}

fn attr_value(e: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes()
        .filter_map(std::result::Result::ok)
        .find(|attr| attr.key.as_ref() == name)
        .map(|attr| String::from_utf8_lossy(attr.value.as_ref()).to_string())
}

fn local_name(name: &[u8]) -> &[u8] {
    name.iter()
        .position(|byte| *byte == b':')
        .map_or(name, |index| &name[index + 1..])
}

fn xml_text(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|raw| unescape(raw).ok().map(std::borrow::Cow::into_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shared_strings_with_rich_text_parts() {
        let xml = r#"<sst><si><t>Alice</t></si><si><r><t>Bob</t></r><r><t> Jones</t></r></si></sst>"#;
        let strings = parse_shared_strings(xml).expect("shared strings");
        assert_eq!(strings, vec!["Alice".to_string(), "Bob Jones".to_string()]);
    }

    #[test]
    fn parses_sheet_rows_with_shared_strings_and_blanks() {
        let xml = r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="C1" t="inlineStr"><is><t>Phone</t></is></c></row><row r="2"><c r="A2" t="s"><v>1</v></c><c r="B2"><v>42</v></c></row></sheetData></worksheet>"#;
        let shared = vec!["Email".to_string(), "alice@example.com".to_string()];
        let rows = parse_sheet_rows(xml, &shared).expect("sheet rows");
        assert_eq!(rows[0], vec!["Email".to_string(), String::new(), "Phone".to_string()]);
        assert_eq!(rows[1], vec!["alice@example.com".to_string(), "42".to_string()]);
    }

    #[test]
    fn parses_column_index() {
        assert_eq!(column_index_from_cell_ref("A1"), Some(0));
        assert_eq!(column_index_from_cell_ref("Z1"), Some(25));
        assert_eq!(column_index_from_cell_ref("AA1"), Some(26));
    }
}
