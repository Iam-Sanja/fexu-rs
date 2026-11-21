# Forms and validation

Forms receive input from the browser, validate it on the server and return either a successful response or an error response. Fexu includes form helpers and rendering; your application supplies the actual business-specific validation.

## Example: a simple HTML form

**File: `apps/catalog/templates/catalog/contact.html`**

```html
{% extends "base.html" %}
{% block content %}
  <h1>Contact</h1>
  <form method="post">
    <label for="name">Name</label>
    <input id="name" name="name" required maxlength="120">
    <button type="submit">Send</button>
  </form>
{% endblock %}
```

This is an HTML/template example, **not** a complete authenticated form handler. For a real state-changing endpoint, validate submitted values on the server and check the expected CSRF token/session context. Never treat HTML `required` or `maxlength` as security validation. Unauthenticated forms must also use the framework's designated browser CSRF protection.

See [views and URLs](views-and-urls.md), [authentication and admin](auth-and-admin.md) and the [CLI](../reference/cli.md).
