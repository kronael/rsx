"""RBAC permission matrix for the user-management surface.

Product-level authorization roles (distinct from trading itself — a
regular trader has none of these). Matrix is spec 7
(specs/2/7-dashboard.md) §4 verbatim, plus `manage_roles`: the spec's
dashboard didn't own role assignment, rsx-auth does, so that one action
isn't in the spec's table.
"""

VIEWER = "viewer"
OPERATOR = "operator"
ADMIN = "admin"
AUDITOR = "auditor"

ROLES = (VIEWER, OPERATOR, ADMIN, AUDITOR)

_MATRIX: dict[str, frozenset[str]] = {
    VIEWER: frozenset({"read_user_state"}),
    OPERATOR: frozenset({"read_user_state", "write_note"}),
    ADMIN: frozenset({
        "read_user_state",
        "read_audit",
        "write_note",
        "adjust_collateral",
        "freeze_upsert",
        "freeze_delete",
        "manage_roles",
    }),
    AUDITOR: frozenset({"read_user_state", "read_audit"}),
}


def can(role: str | None, action: str) -> bool:
    """Deny by default: role=None (no management role) or an action not
    listed for the role never passes, including an unknown action."""
    if role is None:
        return False
    return action in _MATRIX.get(role, frozenset())
