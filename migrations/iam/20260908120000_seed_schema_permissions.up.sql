INSERT INTO iam.modules (name) VALUES ('Schema Module');

INSERT INTO
    iam.permissions (name, resource, module_id)
SELECT
    permission.name,
    'Schemas',
    m.id
FROM
    iam.modules m,
    (VALUES ('View All'), ('Create'), ('Update'), ('Delete')) AS permission(name)
WHERE
    m.name = 'Schema Module';

INSERT INTO iam.role_has_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM iam.roles r, iam.permissions p
WHERE p.resource = 'Schemas' AND (r.name = 'Admin' OR r.name = 'Developer');
