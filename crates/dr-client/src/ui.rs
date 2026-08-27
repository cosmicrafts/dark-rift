// ============================================================================
// Dark Rift // Zero-Overhead Singularity HUD
// Dark Sci-Fi Observatory Telemetry & Interactive Gravitational Field
// ============================================================================

use dr_core::celestial::CelestialType;
use dr_core::game::GameState;
use dr_core::prestige::ShardUpgradeId;
use dr_core::tiers::MatterTierId;
use dr_core::tutorial::TutorialStep;
use dr_core::upgrades::UpgradeId;
use egui::{
    vec2, Align, Align2, Color32, FontId, Frame, Layout, Panel, Pos2, ProgressBar, Rect, RichText,
    Stroke, Ui,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiTab {
    Matter,
    Tech,
    Collapse,
    Logs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuyAmount {
    One,
    Ten,
    Hundred,
    Max,
}

#[derive(Clone, Debug)]
pub struct ClickPopup {
    pub pos: Pos2,
    pub text: String,
    pub color: Color32,
    pub lifetime: f32,
    pub max_lifetime: f32,
}

pub struct GameUi {
    pub current_tab: UiTab,
    pub buy_amount: BuyAmount,
    pub popups: Vec<ClickPopup>,
    pub anim_time: f32,
    pub hovered_body_id: Option<u32>,
}

impl Default for GameUi {
    fn default() -> Self {
        Self {
            current_tab: UiTab::Matter,
            buy_amount: BuyAmount::One,
            popups: Vec::new(),
            anim_time: 0.0,
            hovered_body_id: None,
        }
    }
}

impl GameUi {
    #[allow(deprecated)]
    pub fn draw(&mut self, ctx: &egui::Context, state: &mut GameState) {
        self.anim_time += 0.016;
        self.apply_theme(ctx);
        self.update_popups(0.016);

        let matter_unlocked = state.black_hole.lifetime_mass.to_f64() >= 10.0 || state.tiers[0].count > 0;
        let tech_unlocked = state.black_hole.lifetime_mass.to_f64() >= 150.0 || state.upgrades.iter().any(|u| u.level > 0);
        let prestige_unlocked = state.prestige.pending_shards(state.black_hole.mass) > 0 || state.prestige.total_collapses > 0;
        let logs_unlocked = state.black_hole.lifetime_mass.to_f64() >= 30.0;

        let viewport_rect = ctx.viewport_rect();
        let center_x = viewport_rect.min.x + (viewport_rect.width() - 380.0) * 0.5;
        let center_y = viewport_rect.center().y;
        let bh_center = Pos2::new(center_x, center_y);
        let screen_height = viewport_rect.height().max(1.0);

        let central_frame = Frame::NONE.fill(Color32::TRANSPARENT);
        egui::CentralPanel::default().frame(central_frame).show(ctx, |ui| {
            // 1. Top Status Header Panel
            Panel::top("top_header")
                .frame(Frame::NONE.fill(Color32::from_rgba_unmultiplied(8, 10, 18, 240)).inner_margin(12.0))
                .show_inside(ui, |ui| {
                    self.draw_top_bar(ui, state);
                });

            // 2. Right Control Sidebar
            Panel::right("right_panel")
                .resizable(false)
                .default_size(380.0)
                .frame(Frame::NONE.fill(Color32::from_rgba_unmultiplied(10, 12, 22, 248)).inner_margin(14.0))
                .show_inside(ui, |ui| {
                    if !matter_unlocked && state.tutorial.step == TutorialStep::AwakenSingularity {
                        self.draw_onboarding_guide(ui, state);
                    } else {
                        self.draw_tabs(ui, tech_unlocked, prestige_unlocked, logs_unlocked);
                        ui.add_space(8.0);
                        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                            match self.current_tab {
                                UiTab::Matter => self.draw_matter_tab(ui, state),
                                UiTab::Tech => {
                                    if tech_unlocked {
                                        self.draw_tech_tab(ui, state);
                                    } else {
                                        ui.label(RichText::new("LOCKED // REACH 150 kg TO UNLOCK TECH").font(FontId::proportional(13.0)).color(Color32::GRAY));
                                    }
                                }
                                UiTab::Collapse => {
                                    if prestige_unlocked {
                                        self.draw_collapse_tab(ui, state);
                                    } else {
                                        ui.label(RichText::new("LOCKED // REACH 1,000,000 t MASS TO COLLAPSE").font(FontId::proportional(13.0)).color(Color32::GRAY));
                                    }
                                }
                                UiTab::Logs => {
                                    if logs_unlocked {
                                        self.draw_logs_tab(ui, state);
                                    } else {
                                        ui.label(RichText::new("LOCKED // ACCUMULATE MORE MASS").font(FontId::proportional(13.0)).color(Color32::GRAY));
                                    }
                                }
                            }
                        });
                    }
                });

            // 3. Central Interactive Space Canvas
            let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());

            // Pointer hover tracking
            let mouse_pos = ctx.pointer_hover_pos();
            self.hovered_body_id = None;

            if let Some(mpos) = mouse_pos {
                if rect.contains(mpos) {
                    let norm_x = (mpos.x - bh_center.x) / (screen_height * 0.5);
                    let norm_y = (mpos.y - bh_center.y) / (screen_height * 0.5);

                    // Find closest hovered celestial body
                    for body in state.celestial.bodies.iter() {
                        if !body.is_active {
                            continue;
                        }
                        let dx = norm_x - body.pos[0];
                        let dy = norm_y - body.pos[1];
                        if (dx * dx + dy * dy).sqrt() <= body.visual_size * 2.5 {
                            self.hovered_body_id = Some(body.id);
                            break;
                        }
                    }
                }
            }

            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let norm_x = (pos.x - bh_center.x) / (screen_height * 0.5);
                    let norm_y = (pos.y - bh_center.y) / (screen_height * 0.5);

                    let (gained, body_opt) = state.interact_canvas([norm_x, norm_y]);

                    let (color, label) = match body_opt {
                        Some(CelestialType::QuantumGas) => (Color32::from_rgb(0, 240, 255), format!("+ {}", gained.format_mass_kg())),
                        Some(CelestialType::MicroMeteor) => (Color32::from_rgb(255, 180, 50), format!("+ {} METEOR", gained.format_mass_kg())),
                        Some(CelestialType::DenseAsteroid) => (Color32::from_rgb(255, 120, 50), format!("+ {} ASTEROID", gained.format_mass_kg())),
                        Some(CelestialType::AntimatterShard) => (Color32::from_rgb(255, 80, 240), format!("+ {} ANTIMATTER", gained.format_mass_kg())),
                        Some(CelestialType::ProtoMoon) => (Color32::from_rgb(160, 220, 255), format!("+ {} PLANETOID", gained.format_mass_kg())),
                        None => (Color32::from_rgb(0, 255, 180), format!("+ {}", gained.format_mass_kg())),
                    };

                    self.popups.push(ClickPopup {
                        pos,
                        text: label,
                        color,
                        lifetime: 0.85,
                        max_lifetime: 0.85,
                    });
                }
            }

            // Draw celestial field telemetry overlays, vectors & popups
            self.draw_celestial_overlays(ui, state, rect, bh_center, screen_height);
            self.draw_popups(ui);
        });
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(Color32::from_rgb(220, 232, 250));
        visuals.window_fill = Color32::from_rgb(10, 12, 20);
        visuals.panel_fill = Color32::from_rgb(10, 12, 20);
        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(14, 18, 30);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(18, 24, 40);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(28, 38, 64);
        visuals.widgets.active.bg_fill = Color32::from_rgb(40, 54, 90);
        ctx.set_visuals(visuals);
    }

    fn update_popups(&mut self, dt: f32) {
        for p in &mut self.popups {
            p.lifetime -= dt;
            p.pos.y -= 45.0 * dt;
        }
        self.popups.retain(|p| p.lifetime > 0.0);
    }

    fn draw_popups(&self, ui: &Ui) {
        for p in &self.popups {
            let alpha = ((p.lifetime / p.max_lifetime) * 255.0) as u8;
            let mut col = p.color;
            col = Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), alpha);
            ui.painter().text(
                p.pos,
                Align2::CENTER_CENTER,
                &p.text,
                FontId::proportional(20.0),
                col,
            );
        }
    }

    fn draw_top_bar(&self, ui: &mut Ui, state: &GameState) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("SINGULARITY // EVENT HORIZON TELEMETRY").font(FontId::monospace(11.0)).color(Color32::from_rgb(0, 210, 255)));
                let mass_str = state.black_hole.mass.format_mass_kg();
                ui.label(RichText::new(mass_str).font(FontId::proportional(26.0)).strong().color(Color32::WHITE));
            });

            ui.add_space(32.0);

            ui.vertical(|ui| {
                ui.label(RichText::new("ACCRETION RATE").font(FontId::monospace(11.0)).color(Color32::from_rgb(140, 160, 190)));
                let dps_str = format!("+ {} /s", state.black_hole.mass_rate.format_mass_kg());
                ui.label(RichText::new(dps_str).font(FontId::proportional(18.0)).strong().color(Color32::from_rgb(0, 255, 180)));
            });

            ui.add_space(32.0);

            if state.prestige.total_shards_earned > 0 || state.prestige.dark_matter_shards > 0 {
                ui.vertical(|ui| {
                    ui.label(RichText::new("DARK MATTER SHARDS").font(FontId::monospace(11.0)).color(Color32::from_rgb(255, 100, 220)));
                    let shards_str = format!("+ {} SHARDS", state.prestige.dark_matter_shards);
                    ui.label(RichText::new(shards_str).font(FontId::proportional(18.0)).strong().color(Color32::from_rgb(255, 120, 240)));
                });
                ui.add_space(32.0);
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let spin_pct = (state.black_hole.spin * 100.0) as u32;
                ui.label(RichText::new(format!("KERR SPIN: {}%", spin_pct)).font(FontId::monospace(13.0)).strong().color(Color32::from_rgb(255, 180, 50)));
            });
        });
    }

    fn draw_celestial_overlays(
        &self,
        ui: &mut Ui,
        state: &mut GameState,
        rect: Rect,
        bh_center: Pos2,
        screen_height: f32,
    ) {
        // 1. Draw gravitational field lines and telemetry tags for hovered celestial bodies
        for body in state.celestial.bodies.iter() {
            if !body.is_active {
                continue;
            }

            let bx = bh_center.x + (body.pos[0] * screen_height) * 0.5;
            let by = bh_center.y + (body.pos[1] * screen_height) * 0.5;
            let bpos = Pos2::new(bx, by);

            let is_hovered = self.hovered_body_id == Some(body.id);

            if is_hovered || body.is_captured {
                // Gravitational force vector thread
                let stroke_color = if body.is_captured {
                    Color32::from_rgba_unmultiplied(0, 255, 200, 180)
                } else {
                    Color32::from_rgba_unmultiplied(0, 200, 255, 120)
                };
                ui.painter().line_segment([bpos, bh_center], Stroke::new(1.2_f32, stroke_color));

                // Subtle orbital ring around object
                let ring_radius = body.visual_size * screen_height * 0.6;
                ui.painter().circle_stroke(bpos, ring_radius, Stroke::new(1.5_f32, stroke_color));

                // Clean telemetry label
                let tag_text = match body.body_type {
                    CelestialType::QuantumGas => "QUANTUM GAS // 1.0 kg",
                    CelestialType::MicroMeteor => "METEORITE // 4.0 kg",
                    CelestialType::DenseAsteroid => "ASTEROID // 15.0 kg",
                    CelestialType::AntimatterShard => "ANTIMATTER // 50.0 kg",
                    CelestialType::ProtoMoon => "PLANETOID // 250.0 kg",
                };

                let tag_pos = Pos2::new(bpos.x, bpos.y - ring_radius - 12.0);
                ui.painter().text(
                    tag_pos,
                    Align2::CENTER_CENTER,
                    tag_text,
                    FontId::monospace(11.0),
                    Color32::from_rgb(200, 230, 255),
                );
            }
        }

        // 2. Gravitational Shockwave Ripple Effect
        if state.celestial.shockwave_timer > 0.0 {
            let shock_radius = state.celestial.shockwave_radius * screen_height * 0.5;
            let alpha = ((state.celestial.shockwave_timer / 0.6) * 160.0) as u8;
            ui.painter().circle_stroke(
                bh_center,
                shock_radius,
                Stroke::new(2.5_f32, Color32::from_rgba_unmultiplied(0, 230, 255, alpha)),
            );
        }

        // 3. Phase Callouts & Guidance
        match state.tutorial.step {
            TutorialStep::AwakenSingularity => {
                let badge_pos = Pos2::new(bh_center.x, rect.min.y + 55.0);
                let badge_rect = Rect::from_center_size(badge_pos, vec2(420.0, 52.0));
                ui.painter().rect_filled(badge_rect, 8.0, Color32::from_rgba_unmultiplied(10, 16, 28, 235));
                ui.painter().rect_stroke(badge_rect, 8.0, Stroke::new(1.5_f32, Color32::from_rgb(0, 200, 255)), egui::StrokeKind::Outside);

                let current_mass = state.black_hole.mass.to_f64();
                ui.painter().text(
                    badge_rect.center(),
                    Align2::CENTER_CENTER,
                    format!("ATRAPA LOS ESCOMBROS O EMITE ONDAS GRAVITACIONALES\n[ {:.1} / 10.0 kg ]", current_mass),
                    FontId::proportional(13.0),
                    Color32::WHITE,
                );
            }
            TutorialStep::BuyFirstGenerator => {
                let arrow_bounce = (self.anim_time * 8.0).sin() * 8.0;
                let arrow_target_y = rect.min.y + 115.0;
                let arrow_start = Pos2::new(rect.max.x - 65.0 + arrow_bounce, arrow_target_y);
                ui.painter().arrow(arrow_start, vec2(50.0, 0.0), Stroke::new(4.0_f32, Color32::from_rgb(255, 210, 40)));

                let text_pos = Pos2::new(arrow_start.x - 10.0, arrow_target_y);
                ui.painter().text(
                    text_pos,
                    Align2::RIGHT_CENTER,
                    "[ COMPRA AQUÍ ]",
                    FontId::proportional(15.0),
                    Color32::from_rgb(255, 220, 50),
                );
            }
            TutorialStep::ExplainPassive => {
                let modal_rect = Rect::from_center_size(
                    Pos2::new(bh_center.x, rect.center().y),
                    vec2(460.0, 140.0),
                );
                ui.painter().rect_filled(modal_rect, 10.0, Color32::from_rgba_unmultiplied(12, 18, 32, 250));
                ui.painter().rect_stroke(modal_rect, 10.0, Stroke::new(2.0_f32, Color32::from_rgb(0, 255, 180)), egui::StrokeKind::Outside);

                ui.painter().text(
                    Pos2::new(modal_rect.center().x, modal_rect.min.y + 24.0),
                    Align2::CENTER_CENTER,
                    "¡SUCCIÓN AUTOMÁTICA ACTIVA (+1 kg/s)!",
                    FontId::proportional(16.0),
                    Color32::from_rgb(0, 255, 180),
                );
                ui.painter().text(
                    Pos2::new(modal_rect.center().x, modal_rect.min.y + 60.0),
                    Align2::CENTER_CENTER,
                    "El campo gravitacional ahora atrae escombros orbitales continuamente.\nHaz clic sobre cuerpos celestes para acelerar su absorción.",
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );

                let btn_rect = Rect::from_center_size(
                    Pos2::new(modal_rect.center().x, modal_rect.max.y - 28.0),
                    vec2(160.0, 28.0),
                );
                ui.painter().rect_filled(btn_rect, 5.0, Color32::from_rgb(0, 180, 140));
                ui.painter().text(
                    btn_rect.center(),
                    Align2::CENTER_CENTER,
                    "CONTINUAR",
                    FontId::proportional(12.0),
                    Color32::BLACK,
                );
                if ui.rect_contains_pointer(btn_rect) && ui.input(|i| i.pointer.primary_clicked()) {
                    state.tutorial.dismiss_explanation();
                }
            }
            TutorialStep::Completed => {
                if let Some(quest) = state.quests.current_quest() {
                    let quest_rect = Rect::from_center_size(
                        Pos2::new(bh_center.x, rect.min.y + 35.0),
                        vec2(420.0, 48.0),
                    );

                    ui.painter().rect_filled(quest_rect, 8.0, Color32::from_rgba_unmultiplied(12, 16, 28, 230));
                    ui.painter().rect_stroke(quest_rect, 8.0, Stroke::new(1.2_f32, Color32::from_rgb(0, 180, 255)), egui::StrokeKind::Outside);

                    let frac = quest.progress_fraction();
                    let title_pos = Pos2::new(quest_rect.min.x + 12.0, quest_rect.min.y + 14.0);
                    let desc_pos = Pos2::new(quest_rect.min.x + 12.0, quest_rect.min.y + 30.0);

                    ui.painter().text(title_pos, Align2::LEFT_CENTER, &quest.title, FontId::monospace(11.0), Color32::from_rgb(0, 220, 255));
                    ui.painter().text(desc_pos, Align2::LEFT_CENTER, &quest.instruction, FontId::proportional(11.0), Color32::from_rgb(180, 200, 230));

                    let bar_rect = Rect::from_min_size(Pos2::new(quest_rect.max.x - 90.0, quest_rect.min.y + 16.0), vec2(75.0, 14.0));
                    ui.painter().rect_filled(bar_rect, 3.0, Color32::from_rgb(25, 30, 48));
                    let fill_rect = Rect::from_min_size(bar_rect.min, vec2(bar_rect.width() * frac, bar_rect.height()));
                    ui.painter().rect_filled(fill_rect, 3.0, Color32::from_rgb(0, 255, 180));
                    ui.painter().text(bar_rect.center(), Align2::CENTER_CENTER, format!("{:.0}%", frac * 100.0), FontId::monospace(9.0), Color32::BLACK);
                }
            }
        }

        // Bottom telemetry caption
        let hint_pos = Pos2::new(bh_center.x, rect.max.y - 20.0);
        ui.painter().text(
            hint_pos,
            Align2::CENTER_CENTER,
            "HAZ CLIC EN OBJETOS ORBITALES PARA ABSORBERLOS // CLIC EN EL VACÍO EMITE ONDAS GRAVITACIONALES",
            FontId::monospace(10.0),
            Color32::from_rgba_unmultiplied(160, 190, 220, 140),
        );
    }

    fn draw_onboarding_guide(&self, ui: &mut Ui, state: &GameState) {
        ui.vertical_centered(|ui| {
            ui.add_space(18.0);
            ui.label(RichText::new("ORBITAL RECEPTOR").strong().font(FontId::monospace(14.0)).color(Color32::from_rgb(0, 220, 255)));
            ui.add_space(8.0);
            ui.label(RichText::new("Una micro-singularidad ha colapsado en el vacío profundo. Captura materia circundante para alcanzar masa crítica.")
                .font(FontId::proportional(12.0))
                .color(Color32::from_rgb(170, 185, 215)));
            ui.add_space(14.0);

            Frame::NONE
                .fill(Color32::from_rgb(16, 20, 34))
                .corner_radius(6.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("FASE 1 // DESPERTAR:").strong().font(FontId::monospace(12.0)).color(Color32::from_rgb(255, 200, 80)));
                    ui.add_space(4.0);
                    ui.label(RichText::new("Haz clic en los escombros de gas cuántico en órbita para alimentarla.").font(FontId::proportional(11.0)));
                    ui.add_space(10.0);
                    let mass_num = state.black_hole.mass.to_f64();
                    let progress = (mass_num / 10.0).clamp(0.0, 1.0) as f32;
                    ui.add(ProgressBar::new(progress).text(format!("{:.1} / 10.0 kg", mass_num)));
                });
        });
    }

    fn draw_tabs(&mut self, ui: &mut Ui, tech_unlocked: bool, prestige_unlocked: bool, logs_unlocked: bool) {
        ui.horizontal(|ui| {
            let mut tabs = vec![(UiTab::Matter, "MATTER", true)];
            if tech_unlocked {
                tabs.push((UiTab::Tech, "TECH", true));
            }
            if prestige_unlocked {
                tabs.push((UiTab::Collapse, "COLLAPSE", true));
            }
            if logs_unlocked {
                tabs.push((UiTab::Logs, "LOGS", true));
            }

            for (tab, label, _unlocked) in tabs {
                let is_selected = self.current_tab == tab;
                let text = if is_selected {
                    RichText::new(label).strong().font(FontId::monospace(13.0)).color(Color32::from_rgb(0, 220, 255))
                } else {
                    RichText::new(label).font(FontId::monospace(13.0)).color(Color32::from_rgb(140, 155, 185))
                };

                if ui.selectable_label(is_selected, text).clicked() {
                    self.current_tab = tab;
                }
            }
        });
    }

    fn draw_matter_tab(&mut self, ui: &mut Ui, state: &mut GameState) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("BUY:").font(FontId::monospace(11.0)).color(Color32::from_rgb(130, 145, 170)));
            let amounts = [
                (BuyAmount::One, "1x"),
                (BuyAmount::Ten, "10x"),
                (BuyAmount::Hundred, "100x"),
                (BuyAmount::Max, "MAX"),
            ];
            for (amt, label) in amounts {
                if ui.selectable_label(self.buy_amount == amt, label).clicked() {
                    self.buy_amount = amt;
                }
            }
        });

        ui.separator();

        let discount = state.prestige.cost_discount();
        let mut buy_action: Option<(MatterTierId, u64)> = None;

        for tier in &state.tiers {
            if !tier.is_unlocked {
                continue;
            }

            let count_to_buy = match self.buy_amount {
                BuyAmount::One => 1,
                BuyAmount::Ten => 10,
                BuyAmount::Hundred => 100,
                BuyAmount::Max => tier.max_affordable(state.black_hole.mass, discount).max(1),
            };

            let cost = tier.cost_for(count_to_buy, discount);
            let can_afford = state.black_hole.mass >= cost;

            let is_tutorial_highlight = state.tutorial.step == TutorialStep::BuyFirstGenerator && tier.id == MatterTierId::QuantumDust;

            let border_color = if is_tutorial_highlight {
                let pulse = (self.anim_time * 8.0).sin().abs();
                Color32::from_rgb((255.0 * pulse) as u8, 200, 50)
            } else {
                Color32::from_rgb(32, 40, 60)
            };

            let frame = Frame::NONE
                .fill(Color32::from_rgb(16, 20, 32))
                .corner_radius(6.0)
                .stroke(Stroke::new(if is_tutorial_highlight { 2.5_f32 } else { 1.0_f32 }, border_color))
                .inner_margin(10.0);

            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&tier.name).strong().font(FontId::proportional(14.0)).color(Color32::WHITE));
                        ui.label(RichText::new(format!("Count: {}", tier.count)).font(FontId::monospace(12.0)).color(Color32::from_rgb(120, 200, 255)));
                        ui.label(RichText::new(&tier.description).font(FontId::proportional(11.0)).color(Color32::from_rgb(120, 135, 155)));
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let btn_text = RichText::new(format!("Buy {}\n{}", count_to_buy, cost.format_mass_kg()))
                            .font(FontId::monospace(11.0))
                            .color(if can_afford { Color32::from_rgb(0, 255, 180) } else { Color32::from_rgb(100, 110, 125) });

                        let btn = egui::Button::new(btn_text).min_size(vec2(80.0, 36.0));
                        if ui.add_enabled(can_afford, btn).clicked() {
                            buy_action = Some((tier.id, count_to_buy));
                        }
                    });
                });
            });
            ui.add_space(5.0);
        }

        if let Some((id, amount)) = buy_action {
            state.buy_tier(id, amount);
        }
    }

    fn draw_tech_tab(&self, ui: &mut Ui, state: &mut GameState) {
        ui.label(RichText::new("SINGULARITY TECHNOLOGIES").font(FontId::monospace(13.0)).color(Color32::from_rgb(0, 210, 255)));
        ui.separator();

        let mut buy_action: Option<UpgradeId> = None;

        for upgrade in &state.upgrades {
            if !upgrade.is_unlocked {
                continue;
            }

            let cost = upgrade.cost();
            let is_max = upgrade.level >= upgrade.max_level;
            let can_afford = !is_max && state.black_hole.mass >= cost;

            Frame::NONE
                .fill(Color32::from_rgb(16, 20, 32))
                .corner_radius(6.0)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&upgrade.name).strong().font(FontId::proportional(14.0)).color(Color32::WHITE));
                            ui.label(RichText::new(format!("Level: {} / {}", upgrade.level, upgrade.max_level)).font(FontId::monospace(11.0)).color(Color32::from_rgb(255, 200, 80)));
                            ui.label(RichText::new(&upgrade.description).font(FontId::proportional(11.0)).color(Color32::from_rgb(120, 135, 155)));
                        });

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if is_max {
                                ui.label(RichText::new("MAX").font(FontId::monospace(12.0)).color(Color32::from_rgb(100, 255, 150)));
                            } else {
                                let btn_text = RichText::new(format!("Upgrade\n{}", cost.format_mass_kg()))
                                    .font(FontId::monospace(11.0))
                                    .color(if can_afford { Color32::from_rgb(0, 255, 180) } else { Color32::from_rgb(100, 110, 125) });

                                let btn = egui::Button::new(btn_text).min_size(vec2(80.0, 36.0));
                                if ui.add_enabled(can_afford, btn).clicked() {
                                    buy_action = Some(upgrade.id);
                                }
                            }
                        });
                    });
                });
            ui.add_space(5.0);
        }

        if let Some(id) = buy_action {
            state.buy_upgrade(id);
        }
    }

    fn draw_collapse_tab(&self, ui: &mut Ui, state: &mut GameState) {
        ui.label(RichText::new("THE BIG CRUNCH // COSMIC COLLAPSE").font(FontId::monospace(13.0)).color(Color32::from_rgb(255, 100, 220)));
        ui.separator();

        let pending_shards = state.prestige.pending_shards(state.black_hole.mass);

        Frame::NONE
            .fill(Color32::from_rgb(24, 14, 32))
            .corner_radius(6.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.label(RichText::new("Collapse the singularity into primordial quantum vacuum. Resets mass and matter tiers for permanent Dark Matter Shards.")
                    .font(FontId::proportional(11.0))
                    .color(Color32::from_rgb(210, 175, 220)));

                ui.add_space(6.0);
                ui.label(RichText::new(format!("Current Shards: {} SHARDS", state.prestige.dark_matter_shards)).font(FontId::monospace(14.0)).strong().color(Color32::from_rgb(255, 120, 240)));
                ui.label(RichText::new(format!("Pending on Collapse: + {} SHARDS", pending_shards)).font(FontId::monospace(14.0)).color(Color32::from_rgb(0, 255, 180)));

                ui.add_space(8.0);
                let can_crunch = pending_shards > 0;
                let btn_text = RichText::new(if can_crunch { "TRIGGER THE BIG CRUNCH" } else { "REQUIRES 1,000,000 t MASS" })
                    .font(FontId::monospace(12.0))
                    .strong()
                    .color(Color32::WHITE);

                let btn = egui::Button::new(btn_text).fill(if can_crunch { Color32::from_rgb(170, 20, 110) } else { Color32::from_rgb(36, 26, 45) });
                if ui.add_enabled(can_crunch, btn).clicked() {
                    state.big_crunch();
                }
            });

        ui.add_space(12.0);
        ui.label(RichText::new("PRIMORDIAL SHARDS TREE").font(FontId::monospace(13.0)).color(Color32::from_rgb(255, 120, 240)));
        ui.separator();

        let mut buy_action: Option<ShardUpgradeId> = None;

        for upgrade in &state.prestige.upgrades {
            let cost = upgrade.cost();
            let is_max = upgrade.level >= upgrade.max_level;
            let can_afford = !is_max && state.prestige.dark_matter_shards >= cost;

            Frame::NONE
                .fill(Color32::from_rgb(18, 18, 30))
                .corner_radius(6.0)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&upgrade.name).strong().font(FontId::proportional(14.0)).color(Color32::WHITE));
                            ui.label(RichText::new(format!("Level: {} / {}", upgrade.level, upgrade.max_level)).font(FontId::monospace(11.0)).color(Color32::from_rgb(255, 120, 240)));
                            ui.label(RichText::new(&upgrade.description).font(FontId::proportional(11.0)).color(Color32::from_rgb(120, 135, 155)));
                        });

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if is_max {
                                ui.label(RichText::new("MAX").font(FontId::monospace(12.0)).color(Color32::from_rgb(100, 255, 150)));
                            } else {
                                let btn_text = RichText::new(format!("Unlock\n{} Shards", cost))
                                    .font(FontId::monospace(11.0))
                                    .color(if can_afford { Color32::from_rgb(255, 120, 240) } else { Color32::from_rgb(100, 110, 125) });

                                let btn = egui::Button::new(btn_text).min_size(vec2(80.0, 36.0));
                                if ui.add_enabled(can_afford, btn).clicked() {
                                    buy_action = Some(upgrade.id);
                                }
                            }
                        });
                    });
                });
            ui.add_space(5.0);
        }

        if let Some(id) = buy_action {
            state.buy_shard_upgrade(id);
        }
    }

    fn draw_logs_tab(&self, ui: &mut Ui, state: &GameState) {
        ui.label(RichText::new("OBSERVATORY TELEMETRY LOGS").font(FontId::monospace(13.0)).color(Color32::from_rgb(0, 210, 255)));
        ui.separator();

        let stats = [
            ("Current Mass", state.black_hole.mass.format_mass_kg()),
            ("Accretion Rate", format!("{} /s", state.black_hole.mass_rate.format_mass_kg())),
            ("Lifetime Consumed", state.black_hole.lifetime_mass.format_mass_kg()),
            ("Schwarzschild Radius (Rs)", format!("{} m", state.black_hole.physical_schwarzschild_radius_m().format_compact())),
            ("Hawking Temperature", format!("{} K", state.black_hole.hawking_temperature_k().format_compact())),
            ("Dimensionless Kerr Spin (a*)", format!("{:.3}", state.black_hole.spin)),
            ("Active Celestial Bodies", format!("{}", state.celestial.active_count)),
            ("Total Big Crunches", format!("{}", state.prestige.total_collapses)),
            ("Lifetime Shards", format!("{} SHARDS", state.prestige.total_shards_earned)),
            ("Simulation Time", format!("{:.1} s", state.time_played_secs)),
        ];

        Frame::NONE
            .fill(Color32::from_rgb(16, 20, 32))
            .corner_radius(6.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                for (label, value) in stats {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(label).font(FontId::proportional(12.0)).color(Color32::from_rgb(130, 150, 180)));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(value).font(FontId::monospace(12.0)).color(Color32::WHITE));
                        });
                    });
                    ui.add_space(3.0);
                }
            });
    }
}
