//! Profile creation, deletion, and config-directory confirmation dialogs.

use crate::ui::components::modal::{button_styles, form_input, modal_block, render_modal};
use crate::ui::components::text::truncate_w;
use crate::ui::render::{centered_fixed_rect, display_path};
use crate::ui::App;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Padding, Paragraph, Wrap},
    Frame,
};

/// Profile creation/edit form: agent radio options, text inputs (Name, Config Path), and Save/Cancel buttons.
pub(crate) fn draw_profile_form(f: &mut Frame, app: &App) {
    use crate::model::Agent;
    use crate::ui::FormFocus;

    let Some(form) = &app.profile_form else {
        return;
    };
    let th = &app.theme;
    let title = if form.editing_id.is_some() {
        " Edit Profile "
    } else {
        " Add Profile "
    };

    // One notice line, in descending order of urgency: a validation error, the
    // Antigravity capability limit, why the locked agent row is dim, and finally
    // why Antigravity cannot be picked while creating.
    let (notice_text, notice_color): (String, Color) = if let Some(err) = &form.error {
        (err.clone(), th.error)
    } else if Agent::all()[form.agent_idx] == Agent::Antigravity {
        (
            "Antigravity: config env not supported — usage/resume runs on the default account"
                .to_string(),
            th.muted,
        )
    } else if form.locked() {
        (
            "Agent type is fixed — delete and re-add to change it".to_string(),
            th.muted,
        )
    } else {
        // Antigravity cannot be selected while creating, so the dim state gets a
        // standing explanation rather than a silent unreachable option.
        (
            "Antigravity is not selectable — custom config folders are not supported".to_string(),
            th.muted,
        )
    };

    // Every state fills the notice line, so the dialog keeps one height and the
    // buttons do not shift between the add and edit forms.
    let area = centered_fixed_rect(72, 13, f.area());
    let block = modal_block(title, th.accent).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Agent radio buttons
            Constraint::Length(3), // Name
            Constraint::Length(3), // Config Path
            Constraint::Length(1), // Error / Notice
            Constraint::Length(1), // Padding spacer
            Constraint::Length(1), // Buttons
        ])
        .split(inner);

    // Agent radio layout: (•) Claude   ( ) Codex   ( ) Antigravity
    let agent_focused = form.focus == FormFocus::Agent;
    let mut radio = vec![Span::styled(
        "Agent  ",
        if agent_focused {
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        },
    )];
    for (i, agent) in Agent::all().iter().enumerate() {
        let selected = i == form.agent_idx;
        let mark = if selected { "(•) " } else { "( ) " };
        let style = if form.locked() {
            // The type cannot change, so the row is dim; the selected entry stays
            // bold so the profile's agent is still readable at a glance.
            if selected {
                th.soft_dim().add_modifier(Modifier::BOLD)
            } else {
                th.soft_dim()
            }
        } else if !form.agent_enabled(i) {
            // Antigravity cannot be picked while creating a profile.
            th.soft_dim()
        } else if selected && agent_focused {
            Style::default().fg(th.on_accent).bg(th.accent)
        } else if selected {
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
        } else {
            th.soft_dim()
        };
        radio.push(Span::styled(
            format!("{}{}", mark, crate::profile::agent_display_name(*agent)),
            style,
        ));
        radio.push(Span::raw("   "));
    }
    f.render_widget(Paragraph::new(Line::from(radio)), rows[0]);

    form_input(
        f,
        rows[1],
        " Name ",
        &form.name,
        form.focus == FormFocus::Name,
        "",
        th,
    );
    form_input(
        f,
        rows[2],
        " Config Path ",
        &form.path,
        form.focus == FormFocus::Path,
        "",
        th,
    );

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate_w(&notice_text, rows[3].width as usize),
            Style::default().fg(notice_color),
        ))),
        rows[3],
    );

    // Buttons: highlighted only when focused on the button row.
    let buttons_focused = form.focus == FormFocus::Buttons;
    let (focused_style, unfocused) = button_styles(th);
    let (save_style, cancel_style) = if !buttons_focused {
        (unfocused, unfocused)
    } else if form.save_focused {
        (focused_style, unfocused)
    } else {
        (unfocused, focused_style)
    };
    let buttons = Line::from(vec![
        Span::styled("   Save   ", save_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[5],
    );
}

/// Profile deletion confirmation modal. Explicitly warns that the actual folder is preserved on disk.
pub(crate) fn draw_profile_delete_confirm(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let (name, path, count) = app
        .pending_profile_delete
        .and_then(|idx| app.profiles.profiles.get(idx))
        .map(|p| {
            let count = app.sessions.iter().filter(|s| s.profile_id == p.id).count();
            (p.name.clone(), display_path(&p.path), count)
        })
        .unwrap_or_else(|| ("?".to_string(), "?".to_string(), 0));

    let area = centered_fixed_rect(70, 11, f.area());
    let block = modal_block(" Delete Profile ", th.error).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);
    let inner_w = inner.width as usize;

    let content = vec![
        Line::from(Span::styled(
            truncate_w(&name, inner_w),
            Style::default().fg(th.error).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            truncate_w(&format!("path: {}", path), inner_w),
            Style::default().fg(th.dim),
        )),
        Line::from(Span::styled(
            format!("sessions: {}", count),
            Style::default().fg(th.dim),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "The folder on disk is NOT deleted. Sessions are only removed from the list.",
            th.soft_dim(),
        )),
    ];

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    f.render_widget(Paragraph::new(content).wrap(Wrap { trim: false }), rows[0]);

    let (focused_style, unfocused) = button_styles(th);
    let (cancel_style, delete_style) = if app.delete_ok_focused {
        (unfocused, focused_style)
    } else {
        (focused_style, unfocused)
    };
    let buttons = Line::from(vec![
        Span::styled("  Delete  ", delete_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[2],
    );
}

/// Modal verifying creation of a missing config directory (and execution of login subprocess) upon profile save.
pub(crate) fn draw_profile_dir_confirm(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let (path_str, agent, login) = app
        .profile_form
        .as_ref()
        .map(|form| {
            let agent = crate::model::Agent::all()
                [form.agent_idx.min(crate::model::Agent::all().len() - 1)];
            let path = crate::config::expand(form.path.value.trim());
            let login = crate::profile::login_runnable(agent, &path);
            (display_path(&path), agent, login)
        })
        .unwrap_or(("?".to_string(), crate::model::Agent::Claude, false));

    let area = centered_fixed_rect(70, 12, f.area());
    let block = modal_block(" Create Config Folder ", th.warning).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);
    let inner_w = inner.width as usize;

    let mut content = vec![
        Line::from(Span::styled(
            truncate_w(&path_str, inner_w),
            Style::default().fg(th.warning).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "This folder does not exist. Create it now?",
            th.soft_dim(),
        )),
        Line::from(""),
    ];
    if login {
        content.push(Line::from(Span::styled(
            format!(
                "{} will launch for login after the folder is created.",
                agent.label()
            ),
            th.soft_dim(),
        )));
        content.push(Line::from(Span::styled(
            "Log in, then exit the agent to return to s7s.",
            Style::default().fg(th.dim),
        )));
    } else {
        content.push(Line::from(Span::styled(
            "Only the folder will be created. Antigravity does not support custom config folders — log in manually.",
            th.soft_dim(),
        )));
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    f.render_widget(Paragraph::new(content).wrap(Wrap { trim: false }), rows[0]);

    let (focused_style, unfocused) = button_styles(th);
    let (ok_style, cancel_style) = if app.dir_create_ok_focused {
        (focused_style, unfocused)
    } else {
        (unfocused, focused_style)
    };
    let buttons = Line::from(vec![
        Span::styled("  Create  ", ok_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[2],
    );
}
