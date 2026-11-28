//! Task window. The list, filter, and reorder state live in the library so tests skip the window.

#![deny(unsafe_code)]

use std::path::PathBuf;

use rune::tok::{self, type_};
use rune::{
    App, AppSpec, Color, Cx, El, ScrollSpec, TextEdit, TextStyle, col, el, row, text, text_input,
};
use vaelrun_core::{Move, TaskStore, icons_dir};
use vaelrun_ui::WindowState;

struct Model {
    window: WindowState,
    cwd: TextEdit,
    filter: TextEdit,
    icon: TextEdit,
}

fn placeholder() -> Model {
    let window = WindowState::open(TaskStore::at("tasks.json"), PathBuf::from("icons"))
        .expect("placeholder store");
    Model::from_window(window)
}

impl Model {
    fn from_window(window: WindowState) -> Self {
        Self {
            cwd: TextEdit::new(window.cwd()),
            filter: TextEdit::new(window.filter()),
            icon: TextEdit::new(""),
            window,
        }
    }
}

fn view(s: &Model, cx: &mut Cx<Model>) -> El {
    let dark = cx.system_dark();
    let t = tok::theme(dark);
    let field = TextStyle::new(14.0, 400.0);
    col()
        .key("window")
        .w_full()
        .h_full()
        .fill(t.bg)
        .padding(16.0)
        .gap(8.0)
        .child(text("vaelrun").font(type_::TITLE).color(t.text))
        .child(labeled_field(
            "cwd",
            "Working directory",
            &s.cwd,
            field,
            t.text_secondary,
            |model| &mut model.cwd,
            |model| {
                let next = model.cwd.text.clone();
                let result = model.window.set_cwd(next);
                model.note(result);
            },
        ))
        .child(labeled_field(
            "filter",
            "Filter",
            &s.filter,
            field,
            t.text_secondary,
            |model| &mut model.filter,
            |model| {
                let next = model.filter.text.clone();
                model.window.set_filter(next);
            },
        ))
        .child(task_list(s, t.text, t.text_secondary, t.hover))
        .child(detail(s, t.text, t.text_secondary))
        .child(actions(&s.icon, t.text))
        .child(output_log(s, t.text, t.text_secondary))
}

fn labeled_field(
    key: &'static str,
    placeholder: &'static str,
    edit: &TextEdit,
    style: TextStyle,
    placeholder_color: Color,
    access: impl for<'a> Fn(&'a mut Model) -> &'a mut TextEdit + 'static,
    change: impl Fn(&mut Model) + 'static,
) -> El {
    text_input(edit, style)
        .key(key)
        .w_full()
        .single_line()
        .placeholder(placeholder, placeholder_color)
        .on_edit(access)
        .on_input_change(move |model, _| change(model))
}

fn task_list(s: &Model, text_color: Color, muted: Color, hover: Color) -> El {
    let mut list = col().key("tasks").gap(4.0).min_h(80.0);
    for task in s.window.visible() {
        let name = task.name.clone();
        let key_name = name.clone();
        let mut label = task.name.clone();
        if let Some(description) = &task.description {
            label.push_str(" — ");
            label.push_str(description);
        }
        if let Some(shortcut) = &task.shortcut {
            label.push(' ');
            label.push_str(shortcut);
        }
        let selected = s
            .window
            .selected()
            .is_some_and(|item| item.name == task.name);
        list = list.child(
            el().key(format!("task-{key_name}"))
                .role("button")
                .label(label.clone())
                .w_full()
                .padding(6.0)
                .hover(hover)
                .on_press(move |model: &mut Model| model.window.select_name(&name))
                .child(text(label).font(type_::BODY).color(if selected {
                    text_color
                } else {
                    muted
                })),
        );
    }
    list
}

fn detail(s: &Model, text_color: Color, muted: Color) -> El {
    let Some(task) = s.window.selected() else {
        return text("Select a task").font(type_::BODY).color(muted);
    };
    let mut body = col().key("detail").gap(4.0).child(
        text(task.command.clone())
            .key("command")
            .font(type_::BODY)
            .color(text_color),
    );
    body = body.child(text("Env files").font(type_::LABEL).color(muted));
    for (index, path) in task.env_files.iter().enumerate() {
        body = body.child(env_row(
            format!("file-{index}"),
            path.display().to_string(),
            text_color,
            move |model| {
                let result = model.window.move_file(index, Move::Up);
                model.note(result);
            },
            move |model| {
                let result = model.window.move_file(index, Move::Down);
                model.note(result);
            },
        ));
    }
    body = body.child(text("Env variables").font(type_::LABEL).color(muted));
    for (index, (key, value)) in task.env.iter().enumerate() {
        body = body.child(env_row(
            format!("var-{index}"),
            format!("{key}={value}"),
            text_color,
            move |model| {
                let result = model.window.move_var(index, Move::Up);
                model.note(result);
            },
            move |model| {
                let result = model.window.move_var(index, Move::Down);
                model.note(result);
            },
        ));
    }
    body
}

fn env_row(
    key: String,
    label: String,
    text_color: Color,
    up: impl Fn(&mut Model) + 'static,
    down: impl Fn(&mut Model) + 'static,
) -> El {
    let up_key = format!("{key}-up");
    let down_key = format!("{key}-down");
    row()
        .key(key)
        .gap(8.0)
        .center_y()
        .child(text(label).font(type_::BODY).color(text_color))
        .child(button(up_key, "Up", text_color, up))
        .child(button(down_key, "Down", text_color, down))
}

fn actions(icon: &TextEdit, text_color: Color) -> El {
    row()
        .key("actions")
        .gap(8.0)
        .child(button("add-icon".into(), "Add icon", text_color, |model| {
            let path = model.icon.text.clone();
            let result = model
                .window
                .add_icon_file(std::path::Path::new(&path))
                .map(|_| ());
            model.note(result);
        }))
        .child(button("run".into(), "Run", text_color, |model| {
            let result = model.window.run_selected().map(|_| ());
            model.note(result);
        }))
        .child(
            text_input(icon, TextStyle::new(14.0, 400.0))
                .key("icon-path")
                .w(220.0)
                .single_line()
                .placeholder("Icon file", text_color)
                .on_edit(|model: &mut Model| &mut model.icon),
        )
}

fn output_log(s: &Model, text_color: Color, muted: Color) -> El {
    let mut lines = col().key("lines").gap(2.0);
    if s.window.output().lines().is_empty() {
        lines = lines.child(text("No output").font(type_::BODY).color(muted));
    }
    for (index, line) in s.window.output().lines().iter().enumerate() {
        lines = lines.child(
            text(line.clone())
                .key(format!("line-{index}"))
                .font(type_::BODY)
                .color(text_color),
        );
    }
    let status = if s.window.status().is_empty() {
        String::new()
    } else {
        s.window.status().to_string()
    };
    col()
        .key("output")
        .grow(1.0)
        .min_h(0.0)
        .h(160.0)
        .scroll(ScrollSpec::default())
        .child(lines)
        .child(text(status).key("status").font(type_::LABEL).color(muted))
}

fn button(
    key: String,
    label: &'static str,
    text_color: Color,
    action: impl Fn(&mut Model) + 'static,
) -> El {
    el().key(key)
        .role("button")
        .label(label)
        .padding(6.0)
        .on_press(action)
        .child(text(label).font(type_::LABEL).color(text_color))
}

impl Model {
    fn note(&mut self, result: Result<(), vaelrun_core::Error>) {
        self.window.set_status(match result {
            Ok(()) => String::new(),
            Err(err) => err.to_string(),
        });
    }
}

fn main() {
    let store = match TaskStore::global() {
        Ok(store) => store,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(err.code);
        }
    };
    let icons = icons_dir().unwrap_or_else(|| PathBuf::from(".vaelrun-icons"));
    let window = match WindowState::open(store, icons) {
        Ok(window) => window,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(err.code);
        }
    };
    let spec = AppSpec {
        ..AppSpec::new("vaelrun", (720.0, 560.0), placeholder, view)
    };
    App::new(spec)
        .init(move || Model::from_window(window))
        .run();
}
