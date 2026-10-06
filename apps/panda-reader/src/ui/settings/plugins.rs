use super::Settings;
use crate::services::Command;
use crate::ui::components::text_input;
use crate::ui::window::ReaderWindow;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use panda_plugins::{CommunityPlugin, PluginSummary};
use tokio::sync::oneshot;

#[derive(Clone)]
enum PluginOperation {
    List,
    Reload,
    Import(String),
    Toggle(String, bool),
    Remove(String),
    InstallCommunity(String),
}

impl Settings {
    pub(in crate::ui) fn refresh_plugin_list(&mut self, cx: &mut Context<ReaderWindow>) {
        self.run_plugin_operation(PluginOperation::List, cx);
        self.check_community_plugins(cx);
    }

    pub(in crate::ui) fn check_community_plugins(&mut self, cx: &mut Context<ReaderWindow>) {
        if !self.plugins_checked && !self.plugin_list_loading {
            self.run_plugin_operation(PluginOperation::List, cx);
        }
        if self.community_plugins_loading {
            return;
        }
        let (reply, response) = oneshot::channel();
        self.community_plugins_loading = true;
        self.community_plugin_error = None;
        self.services
            .send(Command::CommunityPluginCatalog { reply });
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Plugin service stopped".into()));
            let _ = this.update(cx, |this, cx| {
                this.settings.community_plugins_loading = false;
                this.settings.community_plugins_checked = true;
                match result {
                    Ok(plugins) => this.settings.community_plugins = plugins,
                    Err(error) => this.settings.community_plugin_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(in crate::ui) fn community_update_count(&self) -> usize {
        self.community_plugins
            .iter()
            .filter(|available| {
                self.plugins
                    .iter()
                    .find(|installed| installed.manifest.id == available.id)
                    .is_some_and(|installed| {
                        is_newer(&available.version, &installed.manifest.version)
                    })
            })
            .count()
    }

    fn run_plugin_operation(&mut self, operation: PluginOperation, cx: &mut Context<ReaderWindow>) {
        let refresh_open_article = !matches!(&operation, PluginOperation::List);
        let is_list = matches!(&operation, PluginOperation::List);
        let (reply, response) = oneshot::channel();
        let command = match operation {
            PluginOperation::List => Command::PluginList { reply },
            PluginOperation::Reload => Command::ReloadPlugins { reply },
            PluginOperation::Import(source) => Command::ImportPlugin { source, reply },
            PluginOperation::Toggle(id, enabled) => {
                Command::SetPluginEnabled { id, enabled, reply }
            }
            PluginOperation::Remove(id) => Command::RemovePlugin { id, reply },
            PluginOperation::InstallCommunity(id) => Command::InstallCommunityPlugin { id, reply },
        };
        self.plugins_loading = true;
        if is_list {
            self.plugin_list_loading = true;
        }
        self.plugin_error = None;
        self.services.send(command);
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Plugin service stopped".into()));
            let _ = this.update(cx, |this, cx| {
                this.settings.plugins_loading = false;
                match result {
                    Ok(plugins) => {
                        this.settings.plugins = plugins;
                        if is_list {
                            this.settings.plugins_checked = true;
                            this.settings.plugin_list_loading = false;
                        }
                        if refresh_open_article {
                            this.reader.showing_translation = false;
                            this.request_prepared_body(cx);
                            this.settings.check_community_plugins(cx);
                        }
                    }
                    Err(error) => {
                        if is_list {
                            this.settings.plugin_list_loading = false;
                        }
                        this.settings.plugin_error = Some(error)
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn browse_plugin_source(&mut self, archive: bool, cx: &mut Context<ReaderWindow>) {
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let source = if archive {
                rfd::AsyncFileDialog::new()
                    .add_filter("Plugin ZIP archive", &["zip"])
                    .pick_file()
                    .await
                    .map(|file| file.path().display().to_string())
            } else {
                rfd::AsyncFileDialog::new()
                    .pick_folder()
                    .await
                    .map(|folder| folder.path().display().to_string())
            };
            if let Some(source) = source {
                let _ = weak.update(cx, |this, cx| {
                    this.settings
                        .run_plugin_operation(PluginOperation::Import(source), cx);
                });
            }
        })
        .detach();
    }

    pub(in crate::ui) fn render_plugins_settings(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let path = self.plugin_path_input.read(cx).value().to_string();
        let mut rows = v_flex().gap_2();
        for plugin in self.plugins.clone() {
            rows = rows.child(self.plugin_row(plugin, cx));
        }
        let mut community_rows = v_flex().gap_2();
        for plugin in self.community_plugins.clone() {
            community_rows = community_rows.child(self.community_plugin_row(plugin, cx));
        }
        v_flex()
            .gap_3()
            .child(self.settings_heading(owner, "Plugins"))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(
                "Import a plugin folder, ZIP archive, or HTTPS ZIP URL. Plugins can only access the current document.",
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(text_input(&self.plugin_path_input).flex_1())
                    .child(Button::new("plugin-browse-folder").small().secondary().label("Browse folder").on_click(cx.listener(|this, _, _, cx| {
                        this.settings.browse_plugin_source(false, cx);
                    })))
                    .child(Button::new("plugin-browse-zip").small().secondary().label("Browse ZIP").on_click(cx.listener(|this, _, _, cx| {
                        this.settings.browse_plugin_source(true, cx);
                    })))
                    .child(Button::new("plugin-import").small().primary().label("Import").on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.run_plugin_operation(PluginOperation::Import(path.clone()), cx);
                    }))),
            )
            .child(h_flex().justify_between().items_center().child(
                div().text_sm().child(if self.plugins_loading { "Loading plugins…".to_owned() } else { format!("{} plugins", self.plugins.len()) }),
            ).child(
                Button::new("plugin-reload").small().secondary().label("Reload").on_click(cx.listener(|this, _, _, cx| {
                    this.settings.run_plugin_operation(PluginOperation::Reload, cx);
                })),
            ))
            .when_some(self.plugin_error.clone(), |view, error| view.child(
                div().text_sm().text_color(cx.theme().danger).child(error),
            ))
            .child(rows)
            .child(div().my_3().border_t_1().border_color(cx.theme().border))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().font_semibold().child("Community plugins"))
                    .child(
                        Button::new("community-plugins-refresh")
                            .small()
                            .secondary()
                            .label(if self.community_plugins_loading {
                                "Checking…"
                            } else {
                                "Check for updates"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings.check_community_plugins(cx);
                            })),
                    ),
            )
            .when_some(self.community_plugin_error.clone(), |view, error| {
                view.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .when(
                self.community_plugins_loading && self.community_plugins.is_empty(),
                |view| view.child(div().text_sm().child("Loading community plugins…")),
            )
            .when(
                self.community_plugins_checked && self.community_plugins.is_empty(),
                |view| view.child(div().text_sm().child("No community plugins found.")),
            )
            .child(community_rows)
    }

    fn community_plugin_row(
        &self,
        plugin: CommunityPlugin,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let id = plugin.id.clone();
        let installed = self
            .plugins
            .iter()
            .find(|item| item.manifest.id == plugin.id);
        let is_update =
            installed.is_some_and(|item| is_newer(&plugin.version, &item.manifest.version));
        let is_current = installed.is_some() && !is_update;
        let compatible = is_compatible(&plugin);
        let label = if !compatible {
            "Requires newer app"
        } else if is_current {
            "Installed"
        } else if is_update {
            "Update"
        } else {
            "Install"
        };
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_2()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .px_3()
            .py_2()
            .child(
                v_flex()
                    .flex_1()
                    .gap_1()
                    .child(div().text_sm().font_semibold().child(plugin.name))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{} · v{}", plugin.id, plugin.version)),
                    ),
            )
            .child(
                Button::new(format!("community-plugin-{id}"))
                    .small()
                    .secondary()
                    .label(label)
                    .when(!is_current && compatible, |button| {
                        button.on_click(cx.listener(move |this, _, _, cx| {
                            this.settings.run_plugin_operation(
                                PluginOperation::InstallCommunity(id.clone()),
                                cx,
                            );
                        }))
                    }),
            )
    }

    fn plugin_row(
        &self,
        plugin: PluginSummary,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let id = plugin.manifest.id.clone();
        let toggle_id = id.clone();
        let remove_id = id.clone();
        let toggle_label = plugin.manifest.name.clone();
        let enabled = plugin.enabled;
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_2()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .px_3()
            .py_2()
            .child(
                v_flex()
                    .flex_1()
                    .gap_1()
                    .child(div().text_sm().font_semibold().child(plugin.manifest.name))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} · {} · {}",
                                id,
                                plugin.manifest.version,
                                plugin.manifest.domains.join(", ")
                            )),
                    )
                    .when_some(plugin.last_error, |view, error| {
                        view.child(div().text_xs().text_color(cx.theme().danger).child(error))
                    }),
            )
            .child(
                Switch::new(format!("plugin-toggle-{toggle_id}"))
                    .checked(enabled)
                    .accessibility_label(toggle_label)
                    .on_change(cx.listener(move |this, checked, _, cx| {
                        this.settings.run_plugin_operation(
                            PluginOperation::Toggle(toggle_id.clone(), *checked),
                            cx,
                        );
                    })),
            )
            .when(!plugin.bundled, |view| {
                view.child(
                    Button::new(format!("plugin-remove-{remove_id}"))
                        .small()
                        .ghost()
                        .label("Remove")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.settings.run_plugin_operation(
                                PluginOperation::Remove(remove_id.clone()),
                                cx,
                            );
                        })),
                )
            })
    }
}

fn is_newer(remote: &str, local: &str) -> bool {
    semver::Version::parse(remote)
        .ok()
        .zip(semver::Version::parse(local).ok())
        .is_some_and(|(remote, local)| remote > local)
}

fn is_compatible(plugin: &CommunityPlugin) -> bool {
    plugin.api_version == 1
        && semver::Version::parse(&plugin.min_app_version)
            .ok()
            .zip(semver::Version::parse(env!("CARGO_PKG_VERSION")).ok())
            .is_some_and(|(minimum, current)| minimum <= current)
}
