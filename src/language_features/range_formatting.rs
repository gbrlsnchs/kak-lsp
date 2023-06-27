use crate::capabilities::{attempt_server_capability, CAPABILITY_RANGE_FORMATTING};
use crate::context::*;
use crate::text_edit::{apply_text_edits_to_buffer, TextEditish};
use crate::types::*;
use lsp_types::request::*;
use lsp_types::*;
use serde::Deserialize;
use url::Url;

pub fn text_document_range_formatting(meta: EditorMeta, params: EditorParams, ctx: &mut Context) {
    let entry = ctx.language_servers.first_key_value().unwrap();
    if meta.fifo.is_none() && !attempt_server_capability(entry, &meta, CAPABILITY_RANGE_FORMATTING)
    {
        return;
    }

    let params = RangeFormattingParams::deserialize(params)
        .expect("Params should follow RangeFormattingParams structure");

    let req_params = params
        .ranges
        .iter()
        .map(|range| DocumentRangeFormattingParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(&meta.buffile).unwrap(),
            },
            range: *range,
            options: params.formatting_options.clone(),
            work_done_progress_params: Default::default(),
        })
        .collect();
    ctx.call::<RangeFormatting, _>(
        meta,
        RequestParams::All(req_params),
        move |ctx, meta, mut results| {
            if let Some((_, results)) = results.pop() {
                let text_edits = results.into_iter().flatten().collect::<Vec<_>>();
                editor_range_formatting(meta, text_edits, ctx)
            }
        },
    );
}

pub fn editor_range_formatting<T: TextEditish<T>>(
    meta: EditorMeta,
    text_edits: Vec<T>,
    ctx: &mut Context,
) {
    let (_, server) = ctx.language_servers.first_key_value().unwrap();
    let cmd = ctx.documents.get(&meta.buffile).and_then(|document| {
        apply_text_edits_to_buffer(
            &meta.client,
            None,
            text_edits,
            &document.text,
            server.offset_encoding,
            false,
        )
    });
    match cmd {
        Some(cmd) => ctx.exec(meta, cmd),
        // Nothing to do, but sending command back to the editor is required to handle case when
        // editor is blocked waiting for response via fifo.
        None => ctx.exec(meta, "nop"),
    }
}
