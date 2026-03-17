use crate::state::UserError;

#[test]
fn user_error_stores_fields() {
    let err = UserError {
        category: "Hash Failed".to_string(),
        message: "game.rom (snes): I/O error".to_string(),
    };
    assert_eq!(err.category, "Hash Failed");
    assert_eq!(err.message, "game.rom (snes): I/O error");
}

#[test]
fn clear_dismisses_all_errors() {
    let mut errors = vec![
        UserError {
            category: "A".into(),
            message: "first".into(),
        },
        UserError {
            category: "B".into(),
            message: "second".into(),
        },
    ];
    assert_eq!(errors.len(), 2);
    errors.clear();
    assert!(errors.is_empty());
}
