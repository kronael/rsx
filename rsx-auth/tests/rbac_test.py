"""Unit tests for the RBAC permission matrix (spec 7 §4). No DB."""
from rsx_auth import rbac


def test_deny_by_default_no_role():
    for action in (
        "read_user_state", "read_audit", "write_note",
        "adjust_collateral", "freeze_upsert", "freeze_delete",
        "manage_roles",
    ):
        assert rbac.can(None, action) is False


def test_deny_by_default_unknown_action():
    for role in rbac.ROLES:
        assert rbac.can(role, "delete_everything") is False


def test_viewer_reads_only():
    assert rbac.can(rbac.VIEWER, "read_user_state") is True
    for action in (
        "read_audit", "write_note", "adjust_collateral",
        "freeze_upsert", "freeze_delete", "manage_roles",
    ):
        assert rbac.can(rbac.VIEWER, action) is False


def test_operator_reads_and_writes_notes_only():
    assert rbac.can(rbac.OPERATOR, "read_user_state") is True
    assert rbac.can(rbac.OPERATOR, "write_note") is True
    for action in (
        "read_audit", "adjust_collateral", "freeze_upsert",
        "freeze_delete", "manage_roles",
    ):
        assert rbac.can(rbac.OPERATOR, action) is False


def test_admin_can_do_everything_in_the_matrix():
    for action in (
        "read_user_state", "read_audit", "write_note",
        "adjust_collateral", "freeze_upsert", "freeze_delete",
        "manage_roles",
    ):
        assert rbac.can(rbac.ADMIN, action) is True


def test_auditor_reads_state_and_audit_only():
    assert rbac.can(rbac.AUDITOR, "read_user_state") is True
    assert rbac.can(rbac.AUDITOR, "read_audit") is True
    for action in (
        "write_note", "adjust_collateral", "freeze_upsert",
        "freeze_delete", "manage_roles",
    ):
        assert rbac.can(rbac.AUDITOR, action) is False


def test_unknown_role_denied():
    assert rbac.can("superuser", "read_user_state") is False
