use std::collections::HashMap;

use crate::capabilities::attempt_server_capability;
use crate::capabilities::CAPABILITY_HOVER;
use crate::context::*;
use crate::diagnostics::format_related_information;
use crate::markup::*;
use crate::position::*;
use crate::types::*;
use crate::util::editor_quote;
use indoc::formatdoc;
use itertools::Itertools;
use lsp_types::request::*;
use lsp_types::*;
use serde::Deserialize;
use url::Url;

pub fn text_document_hover(meta: EditorMeta, params: EditorParams, ctx: &mut Context) {
    let eligible_servers: Vec<_> = ctx
        .language_servers
        .iter()
        .filter(|srv| attempt_server_capability(*srv, &meta, CAPABILITY_HOVER))
        .collect();
    if meta.fifo.is_none() && eligible_servers.is_empty() {
        return;
    }

    let HoverDetails {
        hover_client: maybe_hover_client,
    } = HoverDetails::deserialize(params.clone()).unwrap();

    let hover_type = match maybe_hover_client {
        Some(client) => HoverType::HoverBuffer { client },
        None => HoverType::InfoBox,
    };

    let params = EditorHoverParams::deserialize(params).unwrap();
    let (range, cursor) = parse_kakoune_range(&params.selection_desc);
    let req_params = eligible_servers
        .into_iter()
        .map(|(language_id, srv_settings)| {
            (
                language_id.clone(),
                vec![HoverParams {
                    text_document_position_params: TextDocumentPositionParams {
                        text_document: TextDocumentIdentifier {
                            uri: Url::from_file_path(&meta.buffile).unwrap(),
                        },
                        position: get_lsp_position(srv_settings, &meta.buffile, &cursor, ctx)
                            .unwrap(),
                    },
                    work_done_progress_params: Default::default(),
                }],
            )
        })
        .collect();
    ctx.call::<HoverRequest, _>(
        meta,
        RequestParams::Each(req_params),
        move |ctx: &mut Context, meta, results| {
            editor_hover(
                meta,
                hover_type,
                cursor,
                range,
                params.tabstop,
                results,
                ctx,
            )
        },
    );
}

pub fn editor_hover(
    meta: EditorMeta,
    hover_type: HoverType,
    cursor: KakounePosition,
    range: KakouneRange,
    tabstop: usize,
    results: Vec<(LanguageId, Option<Hover>)>,
    ctx: &mut Context,
) {
    let doc = &ctx.documents[&meta.buffile];
    let lsp_ranges: HashMap<_, _> = results
        .iter()
        .map(|(language_id, _)| {
            let offset_encoding = ctx.language_servers[language_id].offset_encoding;
            (
                language_id,
                kakoune_range_to_lsp(&range, &doc.text, offset_encoding),
            )
        })
        .collect();
    let for_hover_buffer = matches!(hover_type, HoverType::HoverBuffer { .. });
    let diagnostics = ctx.diagnostics.get(&meta.buffile);
    let diagnostics = diagnostics
        .map(|x| {
            x.iter()
                .filter(|(language_id, x)| {
                    lsp_ranges
                        .get(language_id)
                        .filter(|lsp_range| ranges_touch_same_line(x.range, **lsp_range))
                        .is_some()
                })
                .filter(|(_, x)| !x.message.is_empty())
                .map(|(language_id, x)| {
                    let srv_settings = &ctx.language_servers[language_id];
                    // Indent line breaks to the same level as the bullet point
                    let message = (x.message.trim().to_string()
                        + &format_related_information(
                            x,
                            (language_id, srv_settings),
                            srv_settings,
                            ctx,
                        )
                        .map(|s| "\n  ".to_string() + &s)
                        .unwrap_or_default())
                        .replace('\n', "\n  ");
                    if for_hover_buffer {
                        // We are typically creating Markdown, so use a standard Markdown enumerator.
                        return format!("* [{language_id}] {message}");
                    }

                    let face = x
                        .severity
                        .map(|sev| match sev {
                            DiagnosticSeverity::ERROR => FACE_INFO_DIAGNOSTIC_ERROR,
                            DiagnosticSeverity::WARNING => FACE_INFO_DIAGNOSTIC_WARNING,
                            DiagnosticSeverity::INFORMATION => FACE_INFO_DIAGNOSTIC_INFO,
                            DiagnosticSeverity::HINT => FACE_INFO_DIAGNOSTIC_HINT,
                            _ => {
                                warn!("Unexpected DiagnosticSeverity: {:?}", sev);
                                FACE_INFO_DEFAULT
                            }
                        })
                        .unwrap_or(FACE_INFO_DEFAULT);

                    format!(
                        "• [{language_id}] {{{face}}}{}{{{FACE_INFO_DEFAULT}}}",
                        escape_kakoune_markup(&message),
                    )
                })
                .join("\n")
        })
        .unwrap_or_else(String::new);

    let code_lenses = ctx
        .code_lenses
        .get(&meta.buffile)
        .map(|lenses| {
            lenses
                .iter()
                .filter(|(language_id, lens)| {
                    lsp_ranges
                        .get(language_id)
                        .filter(|lsp_range| ranges_touch_same_line(lens.range, **lsp_range))
                        .is_some()
                })
                .map(|(language_id, lens)| {
                    (
                        language_id,
                        lens.command
                            .as_ref()
                            .map(|cmd| cmd.title.as_str())
                            .unwrap_or("(unresolved)"),
                    )
                })
                .map(|(language_id, title)| {
                    if for_hover_buffer {
                        // We are typically creating Markdown, so use a standard Markdown enumerator.
                        return format!("* {}", &title);
                    }
                    format!(
                        "• ({}) {{{}}}{}{{{}}}",
                        language_id,
                        FACE_INFO_DIAGNOSTIC_HINT,
                        escape_kakoune_markup(title),
                        FACE_INFO_DEFAULT,
                    )
                })
                .join("\n")
        })
        .unwrap_or_default();

    let marked_string_to_hover = |ms: MarkedString| {
        if for_hover_buffer {
            match ms {
                MarkedString::String(markdown) => markdown,
                MarkedString::LanguageString(LanguageString { language, value }) => formatdoc!(
                    "```{}
                     {}
                     ```",
                    &language,
                    &value,
                ),
            }
        } else {
            marked_string_to_kakoune_markup(ms)
        }
    };

    let info: Vec<_> = results
        .into_iter()
        .map(|(_, hover)| {
            let (is_markdown, mut contents) = match hover {
                None => (false, "".to_string()),
                Some(hover) => match hover.contents {
                    HoverContents::Scalar(contents) => (true, marked_string_to_hover(contents)),
                    HoverContents::Array(contents) => (
                        true,
                        contents
                            .into_iter()
                            .map(marked_string_to_hover)
                            .filter(|markup| !markup.is_empty())
                            .join(&if for_hover_buffer {
                                "\n---\n".to_string()
                            } else {
                                format!("\n{{{}}}---{{{}}}\n", FACE_INFO_RULE, FACE_INFO_DEFAULT)
                            }),
                    ),
                    HoverContents::Markup(contents) => match contents.kind {
                        MarkupKind::Markdown => (
                            true,
                            if for_hover_buffer {
                                contents.value
                            } else {
                                markdown_to_kakoune_markup(contents.value)
                            },
                        ),
                        MarkupKind::PlainText => (false, contents.value),
                    },
                },
            };

            if !for_hover_buffer && contents.contains('\t') {
                // TODO also expand tabs in the middle.
                contents = contents
                    .split('\n')
                    .map(|line| {
                        let n = line.bytes().take_while(|c| *c == b'\t').count();
                        " ".repeat(tabstop * n) + &line[n..]
                    })
                    .join("\n");
            }

            (is_markdown, contents)
        })
        .filter(|(_, contents)| !contents.is_empty())
        .collect();

    match hover_type {
        HoverType::InfoBox => {
            if info.is_empty() && diagnostics.is_empty() && code_lenses.is_empty() {
                return;
            }

            let command = format!(
                "lsp-show-hover {} %§{}§ %§{}§ %§{}§",
                cursor,
                info.iter()
                    .map(|(_, contents)| contents.replace('§', "§§"))
                    .join("\n---\n"),
                diagnostics.replace('§', "§§"),
                code_lenses.replace('§', "§§"),
            );
            ctx.exec(meta, command);
        }
        HoverType::Modal {
            modal_heading,
            do_after,
        } => {
            show_hover_modal(
                meta,
                ctx,
                modal_heading,
                do_after,
                info.into_iter().map(|(_, contents)| contents).collect(),
                diagnostics,
            );
        }
        HoverType::HoverBuffer { client } => {
            if info.is_empty() && diagnostics.is_empty() {
                return;
            }

            show_hover_in_hover_client(meta, ctx, client, info, diagnostics);
        }
    };
}

fn show_hover_modal(
    meta: EditorMeta,
    ctx: &Context,
    modal_heading: String,
    do_after: String,
    contents: Vec<String>,
    diagnostics: String,
) {
    let contents = contents
        .into_iter()
        .map(|contents| format!("{}\n---\n{}", modal_heading, contents))
        .map(|s| s.replace('§', "§§"))
        .join("\n---\n");
    let command = format!(
        "lsp-show-hover modal %§{}§ %§{}§ ''",
        contents,
        diagnostics.replace('§', "§§"),
    );
    let command = formatdoc!(
        "evaluate-commands %§
             {}
             {}
         §",
        command.replace('§', "§§"),
        do_after.replace('§', "§§")
    );
    ctx.exec(meta, command);
}

fn show_hover_in_hover_client(
    meta: EditorMeta,
    ctx: &Context,
    hover_client: String,
    contents: Vec<(bool, String)>,
    diagnostics: String,
) {
    // NOTE: Should any containing markdown be enough to use markdown?
    let is_markdown = contents.iter().any(|(is_markdown, _)| *is_markdown);
    let contents = contents
        .into_iter()
        .map(|(_, content)| content)
        .join("\n---\n");
    let contents = if diagnostics.is_empty() {
        contents
    } else {
        formatdoc!(
            "{}

             ## Diagnostics
             {}",
            contents,
            diagnostics,
        )
    };

    let command = format!(
        "%[ edit! -scratch *hover*; \
             set-option buffer=*hover* filetype {}; \
             try %[ add-highlighter buffer/lsp_wrap wrap -word ]; \
             execute-keys Rgk \
         ]",
        if is_markdown { "markdown" } else { "''" },
    );

    let command = formatdoc!(
        "set-register dquote {}
         try %[ delete-buffer! *hover* ]
         try %[
             evaluate-commands -client {hover_client} {command}
         ] catch %[
             new %[
                 rename-client {hover_client}
                 evaluate-commands {command}
                 focus {}
             ]
         ]",
        editor_quote(&contents),
        meta.client.as_ref().unwrap(),
    );

    let command = format!(
        "evaluate-commands -save-regs '\"' {}",
        &editor_quote(&command)
    );
    ctx.exec(meta, command);
}
