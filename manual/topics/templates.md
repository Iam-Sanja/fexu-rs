# Templates with Tera

Fexu server-rendered pages use **Tera** templates. You can extend layouts, include fragments and interpolate escaped values. The fullstack generator writes a project-level layout and a template for every new app.

**File: `templates/base.html`** (simplified):

```html
<!doctype html>
<html lang="{{ locale }}">
<head>
  <meta charset="utf-8">
  <title>{{ app_name }}</title>
  <link rel="stylesheet" href="/static/css/app.css">
</head>
<body>
  {% include "partials/header.html" %}
  <main>{% block content %}{% endblock %}</main>
</body>
</html>
```

**File: `apps/catalog/templates/catalog/index.html`**:

```html
{% extends "base.html" %}

{% block content %}
  <h1>Welcome to the bookstore</h1>
  <p>Rendered by Fexu and Tera.</p>
  <a href="/catalog/status">Catalog status</a>
{% endblock %}
```

The generated project usually loads templates from `templates/`; app-local templates can be placed under `apps/<app>/templates/<app>/`. Project templates can override app templates when the app uses the project's supported override mode.

Run `fexu dev` for local source/template changes; it restarts the application rather than acting as a production process manager. See [first-project tutorial](../tutorial/first-project.md).
