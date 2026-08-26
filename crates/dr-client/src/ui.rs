use dr_core::game::GameState;
use dr_core::objects::ObjectType;
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
    Tiers,
    Upgrades,
    Prestige,
    Stats,
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
    pub fps: u32,
    pub popups: Vec<ClickPopup>,
    pub anim_time: f32,
}

impl Default for GameUi {
    fn default() -> Self {
        Self {
            current_tab: UiTab::Tiers,
            buy_amount: BuyAmount::One,
            fps: 60,
            popups: Vec::new(),
            anim_time: 0.0,
        }
    }
}

impl GameUi {
    pub fn draw(&mut self, ctx: &egui::Context, state: &mut GameState) {
        self.anim_time += 0.016;
        self.apply_theme(ctx);
        self.update_popups(0.016);

        let matter_unlocked = state.black_hole.lifetime_mass.to_f64() >= 10.0 || state.tiers[0].count > 0;
        let tech_unlocked = state.black_hole.lifetime_mass.to_f64() >= 150.0 || state.upgrades.iter().any(|u| u.level > 0);
        let prestige_unlocked = state.prestige.pending_shards(state.black_hole.mass) > 0 || state.prestige.total_collapses > 0;
        let stats_unlocked = state.black_hole.lifetime_mass.to_f64() >= 50.0;

        let viewport_rect = ctx.viewport_rect();
        let center_x = viewport_rect.min.x + (viewport_rect.width() - 380.0) * 0.5;
        let center_y = viewport_rect.center().y;
        let bh_center = Pos2::new(center_x, center_y);
        let aspect = viewport_rect.width() / viewport_rect.height().max(1.0);
        let screen_height = viewport_rect.height().max(1.0);

        // Top Status Header Panel
        Panel::top("top_header")
            .frame(Frame::NONE.fill(Color32::from_rgba_unmultiplied(10, 10, 18, 245)).inner_margin(12.0))
            .show(ctx, |ui| {
                self.draw_top_bar(ui, state);
            });

        // Center Interactive Play Area
        let central_frame = Frame::NONE.fill(Color32::TRANSPARENT);
        egui::CentralPanel::default().frame(central_frame).show(ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());

            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    if let Some((gained, obj_type)) = state.click_at_position(
                        [pos.x, pos.y],
                        [bh_center.x, bh_center.y],
                        aspect,
                        screen_height,
                    ) {
                        let popup_color = match obj_type {
                            ObjectType::QuantumPebble => Color32::from_rgb(0, 255, 200),
                            ObjectType::DenseAsteroid => Color32::from_rgb(255, 180, 50),
                            ObjectType::AntimatterShard => Color32::from_rgb(255, 100, 240),
                            ObjectType::ProtoMoon => Color32::from_rgb(180, 220, 255),
                        };

                        self.popups.push(ClickPopup {
                            pos,
                            text: format!("+ {}", gained.format_mass_kg()),
                            color: popup_color,
                            lifetime: 0.85,
                            max_lifetime: 0.85,
                        });
                    }
                }
            }

            // Draw center HUD overlays, interactive object reticles, vector arrows & popups
            self.draw_center_overlays(ui, state, rect, ctx, matter_unlocked, bh_center, aspect, screen_height);
            self.draw_popups(ui);
        });

        // Right Control Sidebar
        Panel::right("right_panel")
            .resizable(false)
            .default_size(380.0)
            .frame(Frame::NONE.fill(Color32::from_rgba_unmultiplied(12, 14, 24, 248)).inner_margin(14.0))
            .show(ctx, |ui| {
                if !matter_unlocked && state.tutorial.step == TutorialStep::ClickFloatingDebris {
                    self.draw_intro_guide(ui, state);
                } else {
                    self.draw_tabs(ui, tech_unlocked, prestige_unlocked, stats_unlocked);
                    ui.add_space(10.0);
                    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                        match self.current_tab {
                            UiTab::Tiers => self.draw_tiers_tab(ui, state),
                            UiTab::Upgrades => {
                                if tech_unlocked {
                                    self.draw_upgrades_tab(ui, state);
                                } else {
                                    ui.label(RichText::new("LOCKED // REACH 150 kg TO UNLOCK TECH").font(FontId::proportional(14.0)).color(Color32::GRAY));
                                }
                            }
                            UiTab::Prestige => {
                                if prestige_unlocked {
                                    self.draw_prestige_tab(ui, state);
                                } else {
                                    ui.label(RichText::new("LOCKED // REACH 1,000,000 t MASS TO COLLAPSE").font(FontId::proportional(14.0)).color(Color32::GRAY));
                                }
                            }
                            UiTab::Stats => {
                                if stats_unlocked {
                                    self.draw_stats_tab(ui, state);
                                } else {
                                    ui.label(RichText::new("LOCKED // ACCUMULATE MORE MASS").font(FontId::proportional(14.0)).color(Color32::GRAY));
                                }
                            }
                        }
                    });
                }
            });
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(Color32::from_rgb(225, 235, 255));
        visuals.window_fill = Color32::from_rgb(12, 14, 24);
        visuals.panel_fill = Color32::from_rgb(12, 14, 24);
        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(16, 20, 34);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(22, 28, 48);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(34, 44, 76);
        visuals.widgets.active.bg_fill = Color32::from_rgb(45, 60, 105);
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
                FontId::proportional(22.0),
                col,
            );
        }
    }

    fn draw_top_bar(&self, ui: &mut Ui, state: &GameState) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("SINGULARITY // DARK RIFT").font(FontId::proportional(12.0)).color(Color32::from_rgb(0, 210, 255)));
                let mass_str = state.black_hole.mass.format_mass_kg();
                ui.label(RichText::new(mass_str).font(FontId::proportional(26.0)).strong().color(Color32::WHITE));
            });

            ui.add_space(28.0);

            ui.vertical(|ui| {
                ui.label(RichText::new("ACCRETION RATE").font(FontId::proportional(12.0)).color(Color32::from_rgb(150, 165, 195)));
                let dps_str = format!("+ {} /s", state.black_hole.mass_rate.format_mass_kg());
                ui.label(RichText::new(dps_str).font(FontId::proportional(18.0)).strong().color(Color32::from_rgb(0, 255, 180)));
            });

            ui.add_space(28.0);

            if state.prestige.total_shards_earned > 0 || state.prestige.dark_matter_shards > 0 {
                ui.vertical(|ui| {
                    ui.label(RichText::new("DARK MATTER SHARDS").font(FontId::proportional(12.0)).color(Color32::from_rgb(255, 100, 220)));
                    let shards_str = format!("+ {} SHARDS", state.prestige.dark_matter_shards);
                    ui.label(RichText::new(shards_str).font(FontId::proportional(18.0)).strong().color(Color32::from_rgb(255, 120, 240)));
                });
                ui.add_space(28.0);
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let spin_pct = (state.black_hole.spin * 100.0) as u32;
                ui.label(RichText::new(format!("SPIN: {}%", spin_pct)).font(FontId::monospace(14.0)).strong().color(Color32::from_rgb(255, 180, 50)));
            });
        });
    }

    fn draw_center_overlays(
        &self,
        ui: &mut Ui,
        state: &mut GameState,
        rect: Rect,
        _ctx: &egui::Context,
        matter_unlocked: bool,
        bh_center: Pos2,
        aspect: f32,
        screen_height: f32,
    ) {
        // 1. Interactive Object Target Reticles & Visual Guidance
        for obj in state.objects.objects.iter() {
            if !obj.is_active {
                continue;
            }

            let screen_x = bh_center.x + (obj.pos[0] * screen_height) / (2.0 * aspect);
            let screen_y = bh_center.y + (obj.pos[1] * screen_height) / 2.0;
            let obj_center = Pos2::new(screen_x, screen_y);

            match obj.obj_type {
                ObjectType::QuantumPebble => {
                    if state.tutorial.step == TutorialStep::ClickFloatingDebris {
                        let pulse = (self.anim_time * 6.0).sin();
                        let radius = 24.0 + pulse * 4.0;
                        ui.painter().circle_stroke(
                            obj_center,
                            radius,
                            Stroke::new(2.0_f32, Color32::from_rgba_unmultiplied(0, 230, 255, 220)),
                        );
                    }
                }
                ObjectType::DenseAsteroid => {
                    let pulse = (self.anim_time * 5.0).sin();
                    let radius = 32.0 + pulse * 5.0;
                    ui.painter().circle_stroke(
                        obj_center,
                        radius,
                        Stroke::new(2.5_f32, Color32::from_rgba_unmultiplied(255, 180, 50, 240)),
                    );
                    ui.painter().text(
                        Pos2::new(obj_center.x, obj_center.y - 42.0),
                        Align2::CENTER_CENTER,
                        format!("ASTEROIDE DENSO [HP: {}]", obj.hp),
                        FontId::proportional(11.0),
                        Color32::from_rgb(255, 200, 80),
                    );
                }
                ObjectType::AntimatterShard => {
                    let pulse = (self.anim_time * 10.0).sin();
                    let radius = 30.0 + pulse * 6.0;
                    ui.painter().circle_stroke(
                        obj_center,
                        radius,
                        Stroke::new(3.0_f32, Color32::from_rgba_unmultiplied(255, 80, 240, 255)),
                    );
                    ui.painter().text(
                        Pos2::new(obj_center.x, obj_center.y - 40.0),
                        Align2::CENTER_CENTER,
                        "¡ANTIMATERIA! [SUCCIÓN TOTAL]",
                        FontId::proportional(12.0),
                        Color32::from_rgb(255, 120, 240),
                    );
                }
                ObjectType::ProtoMoon => {
                    let pulse = (self.anim_time * 4.0).sin();
                    let radius = 55.0 + pulse * 8.0;
                    ui.painter().circle_stroke(
                        obj_center,
                        radius,
                        Stroke::new(3.5_f32, Color32::from_rgba_unmultiplied(100, 220, 255, 255)),
                    );
                    ui.painter().text(
                        Pos2::new(obj_center.x, obj_center.y - 68.0),
                        Align2::CENTER_CENTER,
                        format!("PROTO-LUNA [RESISTENCIA: {} / {}]", obj.hp, obj.max_hp),
                        FontId::proportional(13.0),
                        Color32::from_rgb(180, 230, 255),
                    );
                }
            }
        }

        // 2. TUTORIAL STEPS & STEP-BY-STEP GUIDANCE
        match state.tutorial.step {
            TutorialStep::ClickFloatingDebris => {
                let pulse_anim = (self.anim_time * 5.0).sin();

                // Vector arrow pointing to black hole
                let arrow_start = Pos2::new(bh_center.x, bh_center.y - 110.0 + pulse_anim * 6.0);
                ui.painter().arrow(arrow_start, vec2(0.0, 55.0), Stroke::new(4.5_f32, Color32::from_rgb(0, 230, 255)));

                // Pointer badge
                let badge_pos = Pos2::new(bh_center.x, bh_center.y - 145.0 + pulse_anim * 6.0);
                let badge_rect = Rect::from_center_size(badge_pos, vec2(380.0, 52.0));
                ui.painter().rect_filled(badge_rect, 8.0, Color32::from_rgba_unmultiplied(0, 40, 70, 245));
                ui.painter().rect_stroke(badge_rect, 8.0, Stroke::new(2.0_f32, Color32::from_rgb(0, 220, 255)), egui::StrokeKind::Outside);

                ui.painter().text(
                    badge_rect.center(),
                    Align2::CENTER_CENTER,
                    format!("HAZ CLIC EN LOS ESCOMBROS FLOTANTES\n[ {} / 3 ABSORBIDOS ]", state.tutorial.rocks_caught),
                    FontId::proportional(15.0),
                    Color32::WHITE,
                );
            }
            TutorialStep::ReachTenKg => {
                let pulse_anim = (self.anim_time * 4.0).sin();

                let arrow_start = Pos2::new(bh_center.x, bh_center.y - 110.0 + pulse_anim * 5.0);
                ui.painter().arrow(arrow_start, vec2(0.0, 55.0), Stroke::new(4.5_f32, Color32::from_rgb(0, 255, 180)));

                let badge_pos = Pos2::new(bh_center.x, bh_center.y - 145.0 + pulse_anim * 5.0);
                let badge_rect = Rect::from_center_size(badge_pos, vec2(380.0, 52.0));
                ui.painter().rect_filled(badge_rect, 8.0, Color32::from_rgba_unmultiplied(10, 40, 60, 245));
                ui.painter().rect_stroke(badge_rect, 8.0, Stroke::new(2.0_f32, Color32::from_rgb(0, 255, 180)), egui::StrokeKind::Outside);

                let current = state.black_hole.mass.to_f64();
                ui.painter().text(
                    badge_rect.center(),
                    Align2::CENTER_CENTER,
                    format!("HAZ CLIC EN ESCOMBROS HASTA 10 KG\n[ {:.1} / 10.0 kg ]", current),
                    FontId::proportional(15.0),
                    Color32::from_rgb(0, 255, 180),
                );
            }
            TutorialStep::BuyFirstTier => {
                let banner_pos = Pos2::new(bh_center.x, rect.min.y + 65.0);
                let banner_rect = Rect::from_center_size(banner_pos, vec2(500.0, 58.0));
                ui.painter().rect_filled(banner_rect, 8.0, Color32::from_rgba_unmultiplied(50, 35, 10, 250));
                ui.painter().rect_stroke(banner_rect, 8.0, Stroke::new(2.5_f32, Color32::from_rgb(255, 200, 50)), egui::StrokeKind::Outside);

                let arrow_bounce = (self.anim_time * 8.0).sin() * 10.0;
                ui.painter().text(
                    banner_pos,
                    Align2::CENTER_CENTER,
                    "¡10 KG ALCANZADOS!\nCOMPRA 'GRAVITATIONAL SIPHON' EN EL PANEL DERECHO",
                    FontId::proportional(14.0),
                    Color32::from_rgb(255, 220, 100),
                );

                // Large vector arrow on the right side pointing left into the Buy button
                let arrow_start = Pos2::new(rect.max.x + 20.0 + arrow_bounce, rect.min.y + 160.0);
                ui.painter().arrow(arrow_start, vec2(-55.0, 0.0), Stroke::new(4.5_f32, Color32::from_rgb(255, 210, 40)));

                let text_pos = Pos2::new(arrow_start.x - 65.0, arrow_start.y);
                ui.painter().text(
                    text_pos,
                    Align2::RIGHT_CENTER,
                    "[ COMPRA AQUÍ ]",
                    FontId::proportional(18.0),
                    Color32::from_rgb(255, 220, 50),
                );
            }
            TutorialStep::ExplainPassive => {
                let modal_rect = Rect::from_center_size(
                    Pos2::new(bh_center.x, rect.center().y),
                    vec2(500.0, 160.0),
                );
                ui.painter().rect_filled(modal_rect, 10.0, Color32::from_rgba_unmultiplied(14, 20, 36, 252));
                ui.painter().rect_stroke(modal_rect, 10.0, Stroke::new(2.5_f32, Color32::from_rgb(0, 255, 180)), egui::StrokeKind::Outside);

                ui.painter().text(
                    Pos2::new(modal_rect.center().x, modal_rect.min.y + 28.0),
                    Align2::CENTER_CENTER,
                    "¡SUCCIÓN GRAVITACIONAL ACTIVA (+1 kg/s)!",
                    FontId::proportional(16.0),
                    Color32::from_rgb(0, 255, 180),
                );
                ui.painter().text(
                    Pos2::new(modal_rect.center().x, modal_rect.min.y + 70.0),
                    Align2::CENTER_CENTER,
                    "El campo de gravedad ahora absorbe escombros automáticamente.\nHaz clic en asteroides para acelerar tu crecimiento.",
                    FontId::proportional(13.0),
                    Color32::WHITE,
                );

                let btn_rect = Rect::from_center_size(
                    Pos2::new(modal_rect.center().x, modal_rect.max.y - 32.0),
                    vec2(180.0, 34.0),
                );
                ui.painter().rect_filled(btn_rect, 6.0, Color32::from_rgb(0, 180, 140));
                ui.painter().text(
                    btn_rect.center(),
                    Align2::CENTER_CENTER,
                    "ENTENDIDO [CONTINUAR]",
                    FontId::proportional(13.0),
                    Color32::BLACK,
                );
                if ui.rect_contains_pointer(btn_rect) && ui.input(|i| i.pointer.primary_clicked()) {
                    state.tutorial.dismiss_explanation();
                }
            }
            TutorialStep::CatchAntimatter => {
                let banner_pos = Pos2::new(bh_center.x, rect.min.y + 40.0);
                let banner_rect = Rect::from_center_size(banner_pos, vec2(460.0, 48.0));
                ui.painter().rect_filled(banner_rect, 8.0, Color32::from_rgba_unmultiplied(40, 15, 50, 230));
                ui.painter().rect_stroke(banner_rect, 8.0, Stroke::new(1.5_f32, Color32::from_rgb(255, 100, 240)), egui::StrokeKind::Outside);

                ui.painter().text(
                    banner_pos,
                    Align2::CENTER_CENTER,
                    "¡ATRAPA ESQUIRLAS DE ANTIMATERIA O ASTEROIDES DENSOS!\nDesatan ondas de succión masiva hacia el horizonte de sucesos.",
                    FontId::proportional(12.0),
                    Color32::from_rgb(255, 180, 255),
                );
            }
            TutorialStep::Completed => {
                if let Some(quest) = state.quests.current_quest() {
                    let quest_rect = Rect::from_center_size(
                        Pos2::new(bh_center.x, rect.min.y + 40.0),
                        vec2(440.0, 56.0),
                    );

                    ui.painter().rect_filled(quest_rect, 8.0, Color32::from_rgba_unmultiplied(16, 18, 30, 230));
                    ui.painter().rect_stroke(quest_rect, 8.0, Stroke::new(1.5_f32, Color32::from_rgb(0, 180, 255)), egui::StrokeKind::Outside);

                    let frac = quest.progress_fraction();
                    let title_pos = Pos2::new(quest_rect.min.x + 14.0, quest_rect.min.y + 16.0);
                    let desc_pos = Pos2::new(quest_rect.min.x + 14.0, quest_rect.min.y + 36.0);

                    ui.painter().text(title_pos, Align2::LEFT_CENTER, &quest.title, FontId::proportional(13.0), Color32::from_rgb(0, 220, 255));
                    ui.painter().text(desc_pos, Align2::LEFT_CENTER, &quest.instruction, FontId::proportional(12.0), Color32::from_rgb(200, 215, 240));

                    let bar_rect = Rect::from_min_size(Pos2::new(quest_rect.max.x - 110.0, quest_rect.min.y + 20.0), vec2(95.0, 18.0));
                    ui.painter().rect_filled(bar_rect, 4.0, Color32::from_rgb(30, 35, 55));
                    let fill_rect = Rect::from_min_size(bar_rect.min, vec2(bar_rect.width() * frac, bar_rect.height()));
                    ui.painter().rect_filled(fill_rect, 4.0, Color32::from_rgb(0, 255, 180));
                    ui.painter().text(bar_rect.center(), Align2::CENTER_CENTER, format!("{:.0}%", frac * 100.0), FontId::monospace(11.0), Color32::BLACK);
                }
            }
        }

        // Antimatter Suction Surge Banner
        if state.objects.antimatter_active_timer > 0.0 {
            let banner_rect = Rect::from_center_size(
                Pos2::new(bh_center.x, rect.min.y + 110.0),
                vec2(340.0, 36.0),
            );
            ui.painter().rect_filled(banner_rect, 6.0, Color32::from_rgba_unmultiplied(200, 30, 180, 230));
            ui.painter().text(
                banner_rect.center(),
                Align2::CENTER_CENTER,
                format!("★ ANTIMATTER SURGE ACTIVE ({:.1}s) ★", state.objects.antimatter_active_timer),
                FontId::proportional(13.0),
                Color32::WHITE,
            );
        }

        // Bottom instruction prompt
        let prompt_text = if !matter_unlocked {
            "HAZ CLIC EN LOS OBJETOS PARA ATRAERLOS A LA SINGULARIDAD"
        } else {
            "HAZ CLIC EN ASTEROIDES O EMITE ONDAS DE CHOQUE GRAVITACIONALES"
        };
        let hint_pos = Pos2::new(bh_center.x, rect.max.y - 30.0);
        ui.painter().text(
            hint_pos,
            Align2::CENTER_CENTER,
            prompt_text,
            FontId::proportional(13.0),
            Color32::from_rgba_unmultiplied(200, 230, 255, 170),
        );
    }

    fn draw_intro_guide(&self, ui: &mut Ui, state: &GameState) {
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            ui.label(RichText::new("LA SINGULARIDAD DESPIERTA").strong().font(FontId::proportional(18.0)).color(Color32::from_rgb(0, 220, 255)));
            ui.add_space(10.0);
            ui.label(RichText::new("Un micro agujero negro ha nacido en el vacío. No tiene generadores automáticos.")
                .font(FontId::proportional(13.0))
                .color(Color32::from_rgb(180, 195, 225)));
            ui.add_space(18.0);

            Frame::NONE
                .fill(Color32::from_rgb(20, 24, 38))
                .corner_radius(8.0)
                .inner_margin(14.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("PASO 1 // RECOLECCIÓN:").strong().font(FontId::proportional(14.0)).color(Color32::from_rgb(255, 200, 80)));
                    ui.add_space(6.0);
                    ui.label(RichText::new("Haz clic en los escombros y rocas espaciales para lanzarlos a la singularidad.").font(FontId::proportional(12.0)));
                    ui.add_space(12.0);
                    let mass_num = state.black_hole.mass.to_f64();
                    let progress = (mass_num / 10.0).clamp(0.0, 1.0) as f32;
                    ui.add(ProgressBar::new(progress).text(format!("{:.1} / 10.0 kg", mass_num)));
                });
        });
    }

    fn draw_tabs(&mut self, ui: &mut Ui, tech_unlocked: bool, prestige_unlocked: bool, stats_unlocked: bool) {
        ui.horizontal(|ui| {
            let mut tabs = vec![(UiTab::Tiers, "MATTER", true)];
            if tech_unlocked {
                tabs.push((UiTab::Upgrades, "TECH", true));
            }
            if prestige_unlocked {
                tabs.push((UiTab::Prestige, "COLLAPSE", true));
            }
            if stats_unlocked {
                tabs.push((UiTab::Stats, "LOGS", true));
            }

            for (tab, label, _unlocked) in tabs {
                let is_selected = self.current_tab == tab;
                let text = if is_selected {
                    RichText::new(label).strong().font(FontId::proportional(14.0)).color(Color32::from_rgb(0, 220, 255))
                } else {
                    RichText::new(label).font(FontId::proportional(14.0)).color(Color32::from_rgb(160, 175, 205))
                };

                if ui.selectable_label(is_selected, text).clicked() {
                    self.current_tab = tab;
                }
            }
        });
    }

    fn draw_tiers_tab(&mut self, ui: &mut Ui, state: &mut GameState) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("BUY:").font(FontId::proportional(12.0)).color(Color32::from_rgb(140, 155, 180)));
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

            let is_tutorial_highlight = state.tutorial.step == TutorialStep::BuyFirstTier && tier.id == MatterTierId::QuantumDust;

            let border_color = if is_tutorial_highlight {
                let pulse = (self.anim_time * 8.0).sin().abs();
                Color32::from_rgb((255.0 * pulse) as u8, 200, 50)
            } else {
                Color32::from_rgb(40, 50, 70)
            };

            let frame = Frame::NONE
                .fill(Color32::from_rgb(18, 22, 36))
                .corner_radius(6.0)
                .stroke(Stroke::new(if is_tutorial_highlight { 2.5_f32 } else { 1.0_f32 }, border_color))
                .inner_margin(10.0);

            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&tier.name).strong().font(FontId::proportional(15.0)).color(Color32::WHITE));
                        ui.label(RichText::new(format!("Count: {}", tier.count)).font(FontId::proportional(13.0)).color(Color32::from_rgb(140, 210, 255)));
                        ui.label(RichText::new(&tier.description).font(FontId::proportional(11.0)).color(Color32::from_rgb(130, 140, 160)));
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let btn_text = RichText::new(format!("Buy {}\n{}", count_to_buy, cost.format_mass_kg()))
                            .font(FontId::proportional(12.0))
                            .color(if can_afford { Color32::from_rgb(0, 255, 180) } else { Color32::from_rgb(120, 130, 140) });

                        let btn = egui::Button::new(btn_text).min_size(vec2(85.0, 38.0));
                        if ui.add_enabled(can_afford, btn).clicked() {
                            buy_action = Some((tier.id, count_to_buy));
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }

        if let Some((id, amount)) = buy_action {
            state.buy_tier(id, amount);
        }
    }

    fn draw_upgrades_tab(&self, ui: &mut Ui, state: &mut GameState) {
        ui.label(RichText::new("SINGULARITY TECHNOLOGIES").font(FontId::proportional(14.0)).color(Color32::from_rgb(0, 210, 255)));
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
                .fill(Color32::from_rgb(18, 22, 36))
                .corner_radius(6.0)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&upgrade.name).strong().font(FontId::proportional(14.0)).color(Color32::WHITE));
                            ui.label(RichText::new(format!("Level: {} / {}", upgrade.level, upgrade.max_level)).font(FontId::proportional(12.0)).color(Color32::from_rgb(255, 200, 80)));
                            ui.label(RichText::new(&upgrade.description).font(FontId::proportional(11.0)).color(Color32::from_rgb(130, 140, 160)));
                        });

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if is_max {
                                ui.label(RichText::new("MAX").font(FontId::proportional(13.0)).color(Color32::from_rgb(100, 255, 150)));
                            } else {
                                let btn_text = RichText::new(format!("Upgrade\n{}", cost.format_mass_kg()))
                                    .font(FontId::proportional(12.0))
                                    .color(if can_afford { Color32::from_rgb(0, 255, 180) } else { Color32::from_rgb(120, 130, 140) });

                                let btn = egui::Button::new(btn_text).min_size(vec2(85.0, 38.0));
                                if ui.add_enabled(can_afford, btn).clicked() {
                                    buy_action = Some(upgrade.id);
                                }
                            }
                        });
                    });
                });
            ui.add_space(6.0);
        }

        if let Some(id) = buy_action {
            state.buy_upgrade(id);
        }
    }

    fn draw_prestige_tab(&self, ui: &mut Ui, state: &mut GameState) {
        ui.label(RichText::new("THE BIG CRUNCH // SINGULARITY COLLAPSE").font(FontId::proportional(14.0)).color(Color32::from_rgb(255, 100, 220)));
        ui.separator();

        let pending_shards = state.prestige.pending_shards(state.black_hole.mass);

        Frame::NONE
            .fill(Color32::from_rgb(26, 16, 36))
            .corner_radius(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.label(RichText::new("Collapse the singularity into primordial quantum vacuum. Resets current mass and matter tiers in exchange for permanent Dark Matter Shards.")
                    .font(FontId::proportional(12.0))
                    .color(Color32::from_rgb(220, 180, 230)));

                ui.add_space(8.0);
                ui.label(RichText::new(format!("Current Shards: {} SHARDS", state.prestige.dark_matter_shards)).font(FontId::proportional(15.0)).strong().color(Color32::from_rgb(255, 120, 240)));
                ui.label(RichText::new(format!("Pending on Collapse: + {} SHARDS", pending_shards)).font(FontId::proportional(15.0)).color(Color32::from_rgb(0, 255, 180)));

                ui.add_space(10.0);
                let can_crunch = pending_shards > 0;
                let btn_text = RichText::new(if can_crunch { "TRIGGER THE BIG CRUNCH" } else { "REQUIRES 1,000,000 t MASS" })
                    .font(FontId::proportional(13.0))
                    .strong()
                    .color(Color32::WHITE);

                let btn = egui::Button::new(btn_text).fill(if can_crunch { Color32::from_rgb(180, 20, 120) } else { Color32::from_rgb(40, 30, 50) });
                if ui.add_enabled(can_crunch, btn).clicked() {
                    state.big_crunch();
                }
            });

        ui.add_space(14.0);
        ui.label(RichText::new("PRIMORDIAL SHARDS TREE").font(FontId::proportional(14.0)).color(Color32::from_rgb(255, 120, 240)));
        ui.separator();

        let mut buy_action: Option<ShardUpgradeId> = None;

        for upgrade in &state.prestige.upgrades {
            let cost = upgrade.cost();
            let is_max = upgrade.level >= upgrade.max_level;
            let can_afford = !is_max && state.prestige.dark_matter_shards >= cost;

            Frame::NONE
                .fill(Color32::from_rgb(20, 20, 34))
                .corner_radius(6.0)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&upgrade.name).strong().font(FontId::proportional(14.0)).color(Color32::WHITE));
                            ui.label(RichText::new(format!("Level: {} / {}", upgrade.level, upgrade.max_level)).font(FontId::proportional(12.0)).color(Color32::from_rgb(255, 120, 240)));
                            ui.label(RichText::new(&upgrade.description).font(FontId::proportional(11.0)).color(Color32::from_rgb(130, 140, 160)));
                        });

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if is_max {
                                ui.label(RichText::new("MAX").font(FontId::proportional(13.0)).color(Color32::from_rgb(100, 255, 150)));
                            } else {
                                let btn_text = RichText::new(format!("Unlock\n{} Shards", cost))
                                    .font(FontId::proportional(12.0))
                                    .color(if can_afford { Color32::from_rgb(255, 120, 240) } else { Color32::from_rgb(120, 130, 140) });

                                let btn = egui::Button::new(btn_text).min_size(vec2(85.0, 38.0));
                                if ui.add_enabled(can_afford, btn).clicked() {
                                    buy_action = Some(upgrade.id);
                                }
                            }
                        });
                    });
                });
            ui.add_space(6.0);
        }

        if let Some(id) = buy_action {
            state.buy_shard_upgrade(id);
        }
    }

    fn draw_stats_tab(&self, ui: &mut Ui, state: &GameState) {
        ui.label(RichText::new("COSMIC ANALYTICS & LOGS").font(FontId::proportional(14.0)).color(Color32::from_rgb(0, 210, 255)));
        ui.separator();

        let stats = [
            ("Current Mass", state.black_hole.mass.format_mass_kg()),
            ("Accretion Rate", format!("{} /s", state.black_hole.mass_rate.format_mass_kg())),
            ("Lifetime Consumed", state.black_hole.lifetime_mass.format_mass_kg()),
            ("Physical Radius (Rs)", format!("{} m", state.black_hole.physical_schwarzschild_radius_m().format_compact())),
            ("Hawking Temperature", format!("{} K", state.black_hole.hawking_temperature_k().format_compact())),
            ("Dimensionless Spin (a*)", format!("{:.3}", state.black_hole.spin)),
            ("Gravity Pulses Fired", format!("{}", state.black_hole.total_pulses)),
            ("Total Big Crunches", format!("{}", state.prestige.total_collapses)),
            ("Lifetime Shards", format!("{} SHARDS", state.prestige.total_shards_earned)),
            ("Simulation Time", format!("{:.1} s", state.time_played_secs)),
        ];

        Frame::NONE
            .fill(Color32::from_rgb(18, 22, 36))
            .corner_radius(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                for (label, value) in stats {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(label).font(FontId::proportional(13.0)).color(Color32::from_rgb(140, 160, 190)));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(value).font(FontId::monospace(13.0)).color(Color32::WHITE));
                        });
                    });
                    ui.add_space(4.0);
                }
            });
    }
}
