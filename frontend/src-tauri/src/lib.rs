pub mod analyze;
pub mod builtin;
pub mod collect;
pub mod compare;
pub mod dashboard;
pub mod database;
pub mod discover;
pub mod enrich;
pub mod feed;
pub mod flowsint;
pub mod fresh_install;
pub mod graph;
pub mod investigations;
pub mod live;
pub mod llm;
pub mod net;
pub mod prefs;
pub mod providers;
pub mod rag;
pub mod scandals;
pub mod settings;
pub mod state;
pub mod tray;
pub mod window_state;
#[cfg(test)]
mod live_tests;


use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();
    // Before anything opens the database or the webview
    fresh_install::wipe_once(&context.config().identifier);

    let mut builder = tauri::Builder::default();
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_window(app)));
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            window_state::restore(app.handle());
            if let Err(e) = tray::install(app.handle()) {
                log::error!("Failed to create the tray icon: {e}");
            }
            // The bundled ONNX Runtime (speech detection and voices of live sessions)
            #[cfg(windows)]
            if let Ok(runtime) = app.path().resolve("onnxruntime.dll", tauri::path::BaseDirectory::Resource) {
                match ort::init_from(runtime.to_string_lossy().into_owned()).with_telemetry(false).commit() {
                    Ok(_) => log::info!("ONNX Runtime from {}", runtime.display()),
                    Err(e) => log::error!("Failed to load the ONNX Runtime from {}: {e}", runtime.display()),
                }
            }
            let data_dir = app.path().app_data_dir()?;
            builtin::set_data_dir(data_dir.clone());
            let db_manager = tauri::async_runtime::block_on(database::DatabaseManager::open(&data_dir))
                .map_err(|e| format!("Failed to open the database: {e}"))?;
            app.manage(state::AppState { db_manager });
            let pool = app.state::<state::AppState>().db_manager.pool().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = live::session::recover(&pool).await {
                    log::error!("Failed to close the interrupted live sessions: {e}");
                }
            });
            collect::scheduler::start(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            match event {
                // Closing hides the window: collection goes on from the tray
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Moved(_) => window_state::remember(window.app_handle()),
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            settings::get_ai_settings,
            settings::save_ai_settings,
            settings::save_api_key,
            settings::list_ollama_models,
            settings::test_ai_model,
            providers::list_provider_models,
            builtin::commands::builtin_ai_list_models,
            builtin::commands::builtin_ai_download_model,
            builtin::commands::builtin_ai_cancel_download,
            builtin::commands::builtin_ai_delete_model,
            builtin::commands::builtin_ai_get_recommended_model,
            builtin::commands::builtin_ai_computer_profile,
            prefs::get_collect_settings,
            prefs::save_collect_settings,
            investigations::list_investigations,
            investigations::create_investigation,
            investigations::update_investigation,
            investigations::set_investigation_kind,
            investigations::delete_investigation,
            investigations::list_targets,
            investigations::save_target,
            investigations::delete_target,
            investigations::find_official_matches,
            investigations::link_official,
            investigations::list_sources,
            investigations::add_source,
            investigations::update_source,
            investigations::delete_source,
            investigations::run_source_now,
            investigations::collect_status,
            investigations::list_documents,
            investigations::get_document,
            investigations::delete_document,
            compare::compare_investigations,
            compare::compare_summary,
            dashboard::dashboard,
            dashboard::scoreboard,
            scandals::mark_scandal,
            scandals::list_scandals,
            scandals::delete_scandal,
            feed::list_feed,
            feed::mark_read,
            feed::mark_all_read,
            feed::set_document_flags,
            graph::investigation_graph,
            enrich::enrich_entity,
            discover::get_discovery,
            discover::save_discovery,
            collect::schedule::get_collect_schedule,
            collect::schedule::save_collect_schedule,
            discover::approve_source,
            discover::reject_source,
            discover::suggest_sources,
            discover::websearch::get_websearch_settings,
            discover::websearch::save_websearch_settings,
            discover::websearch::test_websearch,
            flowsint::get_flowsint_settings,
            flowsint::save_flowsint_settings,
            flowsint::test_flowsint,
            flowsint::create_flowsint_account,
            flowsint::push_to_flowsint,
            flowsint::flowsint_sync,
            flowsint::flowsint_local_status,
            flowsint::flowsint_local_install,
            flowsint::flowsint_local_start,
            flowsint::flowsint_local_stop,
            flowsint::flowsint_local_remove,
            flowsint::open_docker,
            analyze::list_claims,
            analyze::set_finding_status,
            analyze::delete_claim,
            analyze::analysis_status,
            rag::search_documents,
            rag::ask_question,
            rag::list_ask_history,
            rag::get_ask_answer,
            live::commands::live_status,
            live::commands::live_save_settings,
            live::commands::live_start,
            live::commands::live_stop,
            live::commands::live_list_sessions,
            live::commands::live_get_session,
            live::commands::live_update_session,
            live::commands::live_delete_session,
            live::commands::live_resolve_voice,
            live::commands::live_set_line_voice,
            live::commands::live_voice_sample,
            live::commands::live_clip,
            live::commands::live_set_check_status,
            live::commands::live_check_now,
            live::commands::live_detect_person,
            live::commands::debate_start,
            live::commands::debate_list,
            live::commands::debate_people,
            live::commands::debate_add_participant,
        ])
        .build(context)
        .expect("error while building tauri application")
        .run(|app, event| match event {
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => tray::show_window(app),
            tauri::RunEvent::Exit => {
                live::session::shutdown();
                builtin::stop_helpers();
                if let Some(state) = app.try_state::<state::AppState>() {
                    tauri::async_runtime::block_on(state.db_manager.cleanup());
                }
            }
            _ => {}
        });
}
