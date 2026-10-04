INSERT INTO iam.modules (name) VALUES ('Catalogues Module'), ('Customer Module');

INSERT INTO iam.permissions (name, resource, module_id)
SELECT permission.name, permission.resource, m.id
FROM iam.modules m,
    (VALUES
        ('Delete', 'Catalogues', 'Catalogues Module'),
        ('Publish', 'Catalogues', 'Catalogues Module'),
        ('Add Tags', 'Catalogues', 'Catalogues Module'),
        ('Make A Copy', 'Catalogues', 'Catalogues Module'),
        ('Restore', 'Catalogues', 'Catalogues Module'),
        ('Delete', 'Customers', 'Customer Module')
    ) AS permission(name, resource, module)
WHERE m.name = permission.module;

INSERT INTO iam.role_has_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM iam.roles r, iam.permissions p
WHERE p.resource IN ('Catalogues', 'Customers') AND (r.name = 'Admin' OR r.name = 'Developer');
