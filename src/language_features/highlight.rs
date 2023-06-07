use crate::capabilities::{attempt_server_capability, CAPABILITY_DOCUMENT_HIGHLIGHT};
use crate::context::{Context, RequestParams};
use crate::position::*;
use crate::types::{
    EditorMeta, EditorParams, KakounePosition, KakouneRange, LanguageId, PositionParams,
};
use crate::util::editor_quote;
use itertools::Itertools;
use lsp_types::{
    request::DocumentHighlightRequest, DocumentHighlight, DocumentHighlightKind,
    DocumentHighlightParams, TextDocumentIdentifier, TextDocumentPositionParams,
};
use serde::Deserialize;
use url::Url;

pub fn text_document_highlight(meta: EditorMeta, params: EditorParams, ctx: &mut Context) {
    let eligible_servers: Vec<_> = ctx
        .language_servers
        .iter()
        .filter(|srv| attempt_server_capability(*srv, &meta, CAPABILITY_DOCUMENT_HIGHLIGHT))
        .collect();
    if meta.fifo.is_none() && eligible_servers.is_empty() {
        return;
    }

    let params = PositionParams::deserialize(params).unwrap();
    let req_params = eligible_servers
        .into_iter()
        .map(|(language_id, srv_settings)| {
            (
                language_id.clone(),
                vec![DocumentHighlightParams {
                    text_document_position_params: TextDocumentPositionParams {
                        text_document: TextDocumentIdentifier {
                            uri: Url::from_file_path(&meta.buffile).unwrap(),
                        },
                        position: get_lsp_position(
                            srv_settings,
                            &meta.buffile,
                            &params.position,
                            ctx,
                        )
                        .unwrap(),
                    },
                    partial_result_params: Default::default(),
                    work_done_progress_params: Default::default(),
                }],
            )
        })
        .collect();
    ctx.call::<DocumentHighlightRequest, _>(
        meta,
        RequestParams::Each(req_params),
        move |ctx: &mut Context, meta, results| {
            if let Some(result) = results.into_iter().find(|(_, v)| v.is_some()) {
                editor_document_highlight(meta, result, params.position, ctx)
            }
        },
    );
}

fn editor_document_highlight(
    meta: EditorMeta,
    result: (LanguageId, Option<Vec<DocumentHighlight>>),
    main_cursor: KakounePosition,
    ctx: &mut Context,
) {
    let (language_id, result) = result;
    let document = ctx.documents.get(&meta.buffile);
    if document.is_none() {
        return;
    }
    let document = document.unwrap();
    let srv_settings = &ctx.language_servers[&language_id];
    let mut ranges = vec![];
    let range_specs = match result {
        Some(highlights) => highlights
            .into_iter()
            .map(|highlight| {
                let range = lsp_range_to_kakoune(
                    &highlight.range,
                    &document.text,
                    srv_settings.offset_encoding,
                );
                ranges.push(range);
                format!(
                    "{}|{}",
                    range,
                    if highlight.kind == Some(DocumentHighlightKind::WRITE) {
                        "ReferenceBind"
                    } else {
                        "Reference"
                    }
                )
            })
            .join(" "),
        None => "".to_string(),
    };
    let mut command = format!(
        "set-option window lsp_references {} {}",
        meta.version, range_specs,
    );
    if !meta.hook {
        command = select_ranges_and(command, ranges, main_cursor);
    }
    ctx.exec(meta, command);
}

fn select_ranges_and(
    command: String,
    ranges: Vec<KakouneRange>,
    main_cursor: KakounePosition,
) -> String {
    let main_selection_range = match ranges
        .iter()
        .find(|range| range.start <= main_cursor && main_cursor <= range.end)
    {
        Some(range) => range,
        None => {
            error!("main cursor lies outside ranges");
            return command;
        }
    };
    if ranges.is_empty() {
        return command;
    }
    let command = format!(
        "select {} {}\n{}",
        main_selection_range,
        ranges.iter().map(|range| format!("{}", range)).join(" "),
        command
    );
    format!("evaluate-commands {}", editor_quote(&command))
}
