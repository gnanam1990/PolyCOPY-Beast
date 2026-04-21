use leptos::prelude::*;
use leptos_meta::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use crate::data::{self, HealthData, MetricsData, PositionData, SignalData};

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    let refresh = RwSignal::new(0u32);
    let (active_tab, set_active_tab) = signal("dashboard");

    // Auto-refresh every 5s via background async loop
    Effect::new(move |_| {
        let refresh_clone = refresh;
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                gloo_timers::future::sleep(std::time::Duration::from_secs(5)).await;
                refresh_clone.update(|n| *n += 1);
            }
        });
    });

    // v3.0: WebSocket event stream for real-time updates
    Effect::new(move |_| {
        let refresh_clone = refresh;
        wasm_bindgen_futures::spawn_local(async move {
            let window = gloo_utils::window();
            let Ok(host) = window.location().host() else { return; };
            let Ok(proto) = window.location().protocol() else { return; };
            let proto = if proto == "https:" { "wss" } else { "ws" };
            let Ok(ws) = web_sys::WebSocket::new(&format!("{}://{}/ws", proto, host)) else { return; };
            let onmessage = Closure::<dyn FnMut(web_sys::MessageEvent)>::new({
                let refresh = refresh_clone;
                move |e: web_sys::MessageEvent| {
                    if e.data().as_string().is_some() {
                        refresh.update(|n| *n += 1);
                    }
                }
            });
            ws.set_onmessage(Some(onmessage.as_ref().dyn_ref().unwrap()));
            onmessage.forget();
        });
    });

    let health_res = LocalResource::new(move || {
        let _ = refresh.get();
        async move { data::fetch_health().await.ok() }
    });
    let metrics_res = LocalResource::new(move || {
        let _ = refresh.get();
        async move { data::fetch_metrics().await.ok() }
    });
    let positions_res = LocalResource::new(move || {
        let _ = refresh.get();
        async move { data::fetch_positions().await.unwrap_or_default() }
    });
    let signals_res = LocalResource::new(move || {
        let _ = refresh.get();
        async move { data::fetch_signals(12).await.unwrap_or_default() }
    });

    let health_sig = Signal::derive(move || health_res.get().as_deref().cloned().flatten());
    let metrics_sig = Signal::derive(move || metrics_res.get().as_deref().cloned().flatten());
    let positions_sig = Signal::derive(move || positions_res.get().as_deref().cloned().unwrap_or_default());
    let signals_sig = Signal::derive(move || signals_res.get().as_deref().cloned().unwrap_or_default());

    view! {
        <Stylesheet id="leptos" href="/style.css"/>
        <Title text="PolyBot v3 // Command Center"/>

        <div class="app-shell">
            <Sidebar active_tab active_set=set_active_tab />
            <div class="main-area">
                <TopBar health=health_sig refresh=refresh />
                <div class="content-scroll">
                    {move || {
                        let tab = active_tab.get();
                        if tab == "positions" {
                            view! { <PositionsTab data=positions_sig /> }.into_any()
                        } else if tab == "signals" {
                            view! { <SignalsTab data=signals_sig /> }.into_any()
                        } else {
                            view! {
                                <DashboardTab
                                    health=health_sig
                                    metrics=metrics_sig
                                    positions=positions_sig
                                    signals=signals_sig
                                    refresh=refresh
                                />
                            }.into_any()
                        }
                    }}
                </div>
            </div>
        </div>
    }
}

#[component]
fn Sidebar(active_tab: ReadSignal<&'static str>, active_set: WriteSignal<&'static str>) -> impl IntoView {
    let nav_item = |label: &'static str, tab: &'static str, icon: &'static str| {
        let is_active = move || active_tab.get() == tab;
        let cls = move || if is_active() { "nav-item active" } else { "nav-item" };
        view! {
            <button class=cls on:click=move |_| active_set.set(tab)>
                <span class="nav-icon">{icon}</span>
                {label}
            </button>
        }
    };

    view! {
        <aside class="sidebar">
            <div class="brand">
                <div class="brand-icon">"P"</div>
                <div class="brand-text">
                    <h1>"PolyBot"</h1>
                    <span>"Live Command Center"</span>
                </div>
            </div>
            <nav class="nav-group">
                {nav_item("Dashboard", "dashboard", "◈")}
                {nav_item("Positions", "positions", "▤")}
                {nav_item("Signals", "signals", "⟡")}
            </nav>
            <div class="sidebar-footer">"v3.0 // Windows-Native"</div>
        </aside>
    }
}

#[component]
fn TopBar(health: Signal<Option<HealthData>>, refresh: RwSignal<u32>) -> impl IntoView {
    view! {
        <header class="top-bar">
            <div class="top-bar-left">
                {move || {
                    let h = health.get();
                    let paused = h.as_ref().map(|v| v.paused).unwrap_or(false);
                    let status_text = if paused { "PAUSED" } else { "ACTIVE" };
                    let status_cls = if paused { "status-pill paused" } else { "status-pill" };
                    view! {
                        <div class=status_cls>
                            <span class="pulse-dot"></span>
                            <span>{status_text}</span>
                        </div>
                    }
                }}
                {move || {
                    let sim = health.get().map(|h| h.simulation).unwrap_or(true);
                    let mode_cls = if sim { "mode-badge" } else { "mode-badge live" };
                    view! { <div class=mode_cls>{if sim { "Simulation" } else { "Live Trading" }}</div> }
                }}
                <span class="uptime-text">
                    {move || {
                        let secs = health.get().map(|h| h.uptime_secs).unwrap_or(0);
                        format!("{:02}h {:02}m {:02}s", secs / 3600, (secs % 3600) / 60, secs % 60)
                    }}
                </span>
            </div>
            <div class="top-bar-right">
                <button class="btn-refresh" on:click=move |_| refresh.update(|n| *n += 1)>"⟳ Refresh"</button>
            </div>
        </header>
    }
}

#[component]
fn DashboardTab(
    health: Signal<Option<HealthData>>,
    metrics: Signal<Option<MetricsData>>,
    positions: Signal<Vec<PositionData>>,
    signals: Signal<Vec<SignalData>>,
    refresh: RwSignal<u32>,
) -> impl IntoView {
    let (toast_msg, set_toast_msg) = signal(String::new());
    let (toast_ok, set_toast_ok) = signal(true);

    let daily_stats_res = LocalResource::new(move || {
        let _ = refresh.get();
        async move { data::fetch_daily_stats().await.unwrap_or_default() }
    });
    let stats_sig = Signal::derive(move || daily_stats_res.get().as_deref().cloned().unwrap_or_default());

    let pause_action = Action::new_local(move |_: &()| {
        async move {
            match gloo_net::http::Request::post("/health/control/pause").send().await {
                Ok(r) if r.ok() => { set_toast_msg.set("Trading paused.".into()); set_toast_ok.set(true); }
                _ => { set_toast_msg.set("Pause request failed.".into()); set_toast_ok.set(false); }
            }
            refresh.update(|n| *n += 1);
        }
    });

    let resume_action = Action::new_local(move |_: &()| {
        async move {
            match gloo_net::http::Request::post("/health/control/resume").send().await {
                Ok(r) if r.ok() => { set_toast_msg.set("Trading resumed.".into()); set_toast_ok.set(true); }
                _ => { set_toast_msg.set("Resume failed — check cooldown.".into()); set_toast_ok.set(false); }
            }
            refresh.update(|n| *n += 1);
        }
    });

    let estop_action = Action::new_local(move |_: &()| {
        async move {
            if let Ok(true) = gloo_utils::window().confirm_with_message("EMERGENCY STOP: This will flatten all open positions immediately. Are you sure?") {
                match gloo_net::http::Request::post("/health/control/emergency-stop").send().await {
                    Ok(r) if r.ok() => { set_toast_msg.set("Emergency Stop executed.".into()); set_toast_ok.set(true); }
                    _ => { set_toast_msg.set("Emergency Stop failed.".into()); set_toast_ok.set(false); }
                }
                refresh.update(|n| *n += 1);
            }
        }
    });

    view! {
        <section class="stats-grid">
            <div class="card fade-in">
                <div class="card-header">
                    <span class="card-title">"Portfolio Balance"</span>
                    <div class="card-icon purple">"◎"</div>
                </div>
                <div class="card-value">{move || health.get().map(|h| format!("${}", h.balance_usd)).unwrap_or_else(|| "-".into())}</div>
                <div class="card-sub">
                    {move || {
                        let dd = health.get().map(|h| h.drawdown_pct.clone()).unwrap_or_default();
                        let dd_f: f64 = dd.parse().unwrap_or(0.0);
                        let cls = if dd_f > 0.0 { "change neg" } else { "change pos" };
                        view! { <span class=cls>{format!("{}% Drawdown", dd_f)}</span> }
                    }}
                </div>
            </div>

            <div class="card fade-in">
                <div class="card-header">
                    <span class="card-title">"Daily P&L"</span>
                    <div class="card-icon cyan">"▲"</div>
                </div>
                <div class="card-value">{move || health.get().map(|h| format!("${}", h.daily_pnl)).unwrap_or_else(|| "-".into())}</div>
                <div class="card-sub">
                    {move || {
                        let pnl = health.get().map(|h| h.daily_pnl.clone()).unwrap_or_default();
                        let pnl_f: f64 = pnl.parse().unwrap_or(0.0);
                        let cls = if pnl_f >= 0.0 { "change pos" } else { "change neg" };
                        let arrow = if pnl_f >= 0.0 { "▲" } else { "▼" };
                        view! { <span class=cls>{format!("{} {:.2}%", arrow, (pnl_f / 1000.0).abs())}</span> }
                    }}
                </div>
            </div>

            <div class="card fade-in">
                <div class="card-header">
                    <span class="card-title">"Open Positions"</span>
                    <div class="card-icon blue">"◉"</div>
                </div>
                <div class="card-value">{move || health.get().map(|h| h.open_positions.to_string()).unwrap_or_else(|| "-".into())}</div>
                <div class="card-sub">
                    {move || {
                        let total = metrics.get().map(|m| m.trades_executed).unwrap_or(0);
                        view! { <span class="change pos">{format!("{} Total Trades", total)}</span> }
                    }}
                </div>
            </div>

            <div class="card fade-in">
                <div class="card-header">
                    <span class="card-title">"Signals Received"</span>
                    <div class="card-icon green">"⟡"</div>
                </div>
                <div class="card-value">{move || health.get().map(|h| h.signals_received.to_string()).unwrap_or_else(|| "-".into())}</div>
                <div class="card-sub">
                    {move || {
                        let proc = health.get().map(|h| h.signals_processed).unwrap_or(0);
                        let skip = metrics.get().map(|m| m.signals_skipped).unwrap_or(0);
                        view! { <span class="change pos">{format!("{} Processed / {} Skipped", proc, skip)}</span> }
                    }}
                </div>
            </div>
        </section>

        <section class="bento-grid">
            <div class="card" style="grid-column: span 1;">
                <div class="card-header">
                    <span class="card-title">"System Health"</span>
                    <div class="card-icon cyan">"◎"</div>
                </div>
                <div class="health-grid">
                    {move || {
                        let h = health.get();
                        let ws = h.as_ref().map(|v| v.ws_connected).unwrap_or(false);
                        let rpc = h.as_ref().map(|v| v.rpc_status.clone()).unwrap_or_default();
                        let last = h.as_ref().and_then(|v| v.last_signal_at.clone()).unwrap_or_else(|| "Never".into());
                        let stops = h.as_ref().map(|v| v.emergency_stops).unwrap_or(0);
                        let uptime = h.map(|v| v.uptime_secs).unwrap_or(0);

                        view! {
                            <div class="health-item">
                                <span class="health-label">"CLOB WebSocket"</span>
                                <span class="health-value">
                                    {if ws {
                                        view! { <span class="pulse-dot" style="color: var(--success)"></span> <span style="color: var(--success)">"Connected"</span> }.into_any()
                                    } else {
                                        view! { <span class="pulse-dot" style="color: var(--danger)"></span> <span style="color: var(--danger)">"Disconnected"</span> }.into_any()
                                    }}
                                </span>
                            </div>
                            <div class="health-item">
                                <span class="health-label">"RPC Status"</span>
                                <span class="health-value" style={if rpc == "healthy" { "color: var(--success)" } else { "color: var(--danger)" }}>{rpc.clone()}</span>
                            </div>
                            <div class="health-item">
                                <span class="health-label">"Data API Latency"</span>
                                <span class="health-value" style="font-family: var(--font-mono);">
                                    {move || {
                                        let ms = health.get().map(|h| h.data_api_latency_ms).unwrap_or(0);
                                        let color = if ms > 2000 { "var(--danger)" } else if ms > 1000 { "var(--warning)" } else { "var(--success)" };
                                        view! { <span style={format!("color: {}", color)}>{format!("{} ms", ms)}</span> }
                                    }}
                                </span>
                            </div>
                            <div class="health-item">
                                <span class="health-label">"Last Signal"</span>
                                <span class="health-value" style="font-family: var(--font-mono); font-size: 0.8rem;">{last}</span>
                            </div>
                            <div class="health-item">
                                <span class="health-label">"Uptime"</span>
                                <span class="health-value" style="font-family: var(--font-mono);">{format!("{:02}:{:02}:{:02}", uptime / 3600, (uptime % 3600) / 60, uptime % 60)}</span>
                            </div>
                            <div class="health-item">
                                <span class="health-label">"Emergency Stops"</span>
                                <span class="health-value" style={if stops > 0 { "color: var(--danger)" } else { "" }}>{stops.to_string()}</span>
                            </div>
                            <div class="health-item">
                                <span class="health-label">"Execution Latency"</span>
                                <span class="health-value">
                                    {move || {
                                        let m = metrics.get();
                                        let avg = m.as_ref().map(|v| v.avg_latency_us).unwrap_or(0);
                                        let max = m.as_ref().map(|v| v.max_latency_us).unwrap_or(0);
                                        view! {
                                            <span style="font-family: var(--font-mono); color: var(--text-secondary);">
                                                {format!("avg {:} µs / max {:} µs", avg, max)}
                                            </span>
                                        }
                                    }}
                                </span>
                            </div>
                            <div class="health-item">
                                <span class="health-label">"Drawdown"</span>
                                <span class="health-value">
                                    {move || {
                                        let dd = health.get().map(|h| h.drawdown_pct.clone()).unwrap_or_default();
                                        let dd_f: f64 = dd.parse().unwrap_or(0.0);
                                        view! {
                                            <span style={if dd_f > 5.0 { "color: var(--danger)" } else { "color: var(--text-primary)" }}>{format!("{:.2}%", dd_f)}</span>
                                            <div class="progress-track">
                                                <div class="progress-fill" style={format!("width: {}%; background: {}", (dd_f * 5.0).min(100.0), if dd_f > 5.0 { "var(--danger)" } else { "var(--warning)" })}></div>
                                            </div>
                                        }
                                    }}
                                </span>
                            </div>
                        }
                    }}
                </div>
            </div>

            <div class="card">
                <div class="card-header">
                    <span class="card-title">"Execution Summary"</span>
                    <div class="card-icon green">"▲"</div>
                </div>
                {move || match metrics.get() {
                    Some(m) => view! {
                        <div class="health-grid">
                            <div class="health-item">
                                <span class="health-label">"Signals Processed"</span>
                                <span class="health-value">{m.signals_processed.to_string()}</span>
                            </div>
                            <div class="health-item">
                                <span class="health-label">"Signals Skipped"</span>
                                <span class="health-value" style="color: var(--warning)">{m.signals_skipped.to_string()}</span>
                            </div>
                            <div class="health-item">
                                <span class="health-label">"Trades Executed"</span>
                                <span class="health-value" style="color: var(--success)">{m.trades_executed.to_string()}</span>
                            </div>
                            <div class="health-item">
                                <span class="health-label">"Open Positions"</span>
                                <span class="health-value">{m.open_positions.to_string()}</span>
                            </div>
                            <div class="health-item">
                                <span class="health-label">"Daily PnL"</span>
                                <span class="health-value" style={if m.daily_pnl_usd >= 0.0 { "color: var(--success)" } else { "color: var(--danger)" }}>{format!("${:.2}", m.daily_pnl_usd)}</span>
                            </div>
                            <div class="health-item">
                                <span class="health-label">"Drawdown"</span>
                                <span class="health-value" style={if m.current_drawdown_pct > 0.05 { "color: var(--danger)" } else { "color: var(--text-primary)" }}>{format!("{:.2}%", m.current_drawdown_pct * 100.0)}</span>
                            </div>
                        </div>
                    }.into_any(),
                    None => view! {
                        <div class="empty-state">
                            <span style="font-size: 1.5rem; opacity: 0.3;">"◉"</span>
                            <p>"Metrics unavailable"</p>
                        </div>
                    }.into_any()
                }}
            </div>

            <div class="card">
                <div class="card-header">
                    <span class="card-title">"Daily Stats"</span>
                    <div class="card-icon blue">"◈"</div>
                </div>
                {move || {
                    let entries = stats_sig.get();
                    if entries.is_empty() {
                        view! {
                            <div class="empty-state">
                                <span style="font-size: 1.5rem; opacity: 0.3;">"◈"</span>
                                <p>"No daily history yet"</p>
                            </div>
                        }.into_any()
                    } else {
                        let max_pnl: f64 = entries.iter()
                            .filter_map(|e| e.realized_pnl.parse::<f64>().ok())
                            .map(|v| v.abs())
                            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                            .unwrap_or(1.0)
                            .max(1.0);
                        view! {
                            <div class="health-grid">
                                {
                                    entries.iter().map(|e| {
                                        let pnl: f64 = e.realized_pnl.parse().unwrap_or(0.0);
                                        let pct = (pnl.abs() / max_pnl * 100.0).min(100.0);
                                        let color = if pnl >= 0.0 { "var(--success)" } else { "var(--danger)" };
                                        let bar_style = format!("width: {:.0}%; height: 6px; background: {}; border-radius: 3px; margin-top: 4px;", pct, color);
                                        view! {
                                            <div class="health-item" style="flex-direction: column; align-items: flex-start;">
                                                <div style="display: flex; justify-content: space-between; width: 100%;">
                                                    <span class="health-label" style="font-size: 0.75rem;">{e.date.clone()}</span>
                                                    <span class="health-value" style={format!("font-size: 0.8rem; color: {}", color)}>{format!("${:.2}", pnl)}</span>
                                                </div>
                                                <div style="width: 100%; background: rgba(255,255,255,0.05); border-radius: 3px;">
                                                    <div style={bar_style}></div>
                                                </div>
                                            </div>
                                        }
                                    }).collect_view()
                                }
                            </div>
                        }.into_any()
                    }
                }}
            </div>

            <div class="card">
                <div class="card-header">
                    <span class="card-title">"Operator Controls"</span>
                    <div class="card-icon red">"⚠"</div>
                </div>
                <div class="control-group">
                    <div class="control-row">
                        <button class="btn btn-secondary" on:click=move |_| { pause_action.dispatch(()); }>"⏸ Pause"</button>
                        <button class="btn btn-secondary" on:click=move |_| { resume_action.dispatch(()); }>"▶ Resume"</button>
                    </div>
                    <div class="control-row">
                        <button class="btn btn-danger" on:click=move |_| { estop_action.dispatch(()); }>"⏹ Emergency Stop"</button>
                    </div>
                    {move || {
                        let msg = toast_msg.get();
                        if msg.is_empty() {
                            view! { <div></div> }.into_any()
                        } else {
                            let cls = if toast_ok.get() { "toast ok" } else { "toast err" };
                            view! { <div class=cls>{msg}</div> }.into_any()
                        }
                    }}
                </div>
            </div>
        </section>

        <section class="tables-grid">
            <div class="card">
                <div class="card-header">
                    <span class="card-title">"Open Positions"</span>
                    {move || {
                        let count = positions.get().len();
                        view! { <span style="font-size: 0.8rem; color: var(--text-tertiary); font-weight: 600;">{format!("{} Active", count)}</span> }
                    }}
                </div>
                <PositionsTable data=positions />
            </div>
            <div class="card">
                <div class="card-header">
                    <span class="card-title">"Recent Signals"</span>
                    {move || {
                        let count = signals.get().len();
                        view! { <span style="font-size: 0.8rem; color: var(--text-tertiary); font-weight: 600;">{format!("{} Items", count)}</span> }
                    }}
                </div>
                <SignalsTable data=signals />
            </div>
        </section>
    }
}

#[component]
fn PositionsTab(data: Signal<Vec<PositionData>>) -> impl IntoView {
    view! {
        <div class="card">
            <div class="card-header">
                <span class="card-title">"All Positions"</span>
                {move || view! { <span style="font-size: 0.8rem; color: var(--text-tertiary);">{format!("{} Records", data.get().len())}</span> }}
            </div>
            <PositionsTable data=data />
        </div>
    }
}

#[component]
fn SignalsTab(data: Signal<Vec<SignalData>>) -> impl IntoView {
    view! {
        <div class="card">
            <div class="card-header">
                <span class="card-title">"Signal History"</span>
                {move || view! { <span style="font-size: 0.8rem; color: var(--text-tertiary);">{format!("{} Records", data.get().len())}</span> }}
            </div>
            <SignalsTable data=data />
        </div>
    }
}

#[component]
fn PositionsTable(data: Signal<Vec<PositionData>>) -> impl IntoView {
    view! {
        {move || {
            let rows = data.get();
            if rows.is_empty() {
                view! {
                    <div class="empty-state">
                        <span style="font-size: 2rem; opacity: 0.2;">"▤"</span>
                        <p>"No open positions"</p>
                    </div>
                }.into_any()
            } else {
                view! {
                    <div style="overflow-x: auto;">
                        <table class="data-table">
                            <thead>
                                <tr>
                                    <th>"Market"</th>
                                    <th>"Side"</th>
                                    <th>"Avg Price"</th>
                                    <th>"Size"</th>
                                    <th>"Category"</th>
                                    <th>"Status"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {rows.into_iter().map(|p| {
                                    let side_tag = if p.side.to_lowercase() == "yes" || p.side.to_lowercase() == "buy" {
                                        view! { <span class="tag tag-yes">{p.side}</span> }.into_any()
                                    } else {
                                        view! { <span class="tag tag-no">{p.side}</span> }.into_any()
                                    };
                                    let cat_tag = match p.category.to_lowercase().as_str() {
                                        "politics" => view! { <span class="tag tag-politics">{p.category}</span> }.into_any(),
                                        "crypto" => view! { <span class="tag tag-crypto">{p.category}</span> }.into_any(),
                                        "sports" => view! { <span class="tag tag-sports">{p.category}</span> }.into_any(),
                                        _ => view! { <span class="tag tag-other">{p.category}</span> }.into_any(),
                                    };
                                    let status_tag = if p.status.to_lowercase() == "open" {
                                        view! { <span class="tag tag-open">{p.status}</span> }.into_any()
                                    } else {
                                        view! { <span class="tag tag-closed">{p.status}</span> }.into_any()
                                    };
                                    view! {
                                        <tr class="fade-in">
                                            <td style="font-weight: 600;">{p.market_id}</td>
                                            <td>{side_tag}</td>
                                            <td class="td-mono">{p.average_price}</td>
                                            <td class="td-mono">{p.current_size}</td>
                                            <td>{cat_tag}</td>
                                            <td>{status_tag}</td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    </div>
                }.into_any()
            }
        }}
    }
}

#[component]
fn SignalsTable(data: Signal<Vec<SignalData>>) -> impl IntoView {
    view! {
        {move || {
            let rows = data.get();
            if rows.is_empty() {
                view! {
                    <div class="empty-state">
                        <span style="font-size: 2rem; opacity: 0.2;">"⟡"</span>
                        <p>"No recent signals"</p>
                    </div>
                }.into_any()
            } else {
                view! {
                    <div style="overflow-x: auto;">
                        <table class="data-table">
                            <thead>
                                <tr>
                                    <th>"Market"</th>
                                    <th>"Side"</th>
                                    <th>"Conf"</th>
                                    <th>"Secret"</th>
                                    <th>"Category"</th>
                                    <th>"Disposition"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {rows.into_iter().map(|s| {
                                    let side_tag = if s.side.to_lowercase() == "yes" || s.side.to_lowercase() == "buy" {
                                        view! { <span class="tag tag-yes">{s.side}</span> }.into_any()
                                    } else {
                                        view! { <span class="tag tag-no">{s.side}</span> }.into_any()
                                    };
                                    let cat_tag = match s.category.to_lowercase().as_str() {
                                        "politics" => view! { <span class="tag tag-politics">{s.category}</span> }.into_any(),
                                        "crypto" => view! { <span class="tag tag-crypto">{s.category}</span> }.into_any(),
                                        "sports" => view! { <span class="tag tag-sports">{s.category}</span> }.into_any(),
                                        _ => view! { <span class="tag tag-other">{s.category}</span> }.into_any(),
                                    };
                                    let disp_tag = if s.disposition.to_lowercase() == "execute" {
                                        view! { <span class="tag tag-open">{s.disposition}</span> }.into_any()
                                    } else {
                                        view! { <span class="tag tag-closed">{s.disposition}</span> }.into_any()
                                    };
                                    view! {
                                        <tr class="fade-in">
                                            <td style="font-weight: 600;">{s.market_id}</td>
                                            <td>{side_tag}</td>
                                            <td class="td-mono">{s.confidence.to_string()}</td>
                                            <td class="td-mono">{s.secret_level.to_string()}</td>
                                            <td>{cat_tag}</td>
                                            <td>{disp_tag}</td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    </div>
                }.into_any()
            }
        }}
    }
}
