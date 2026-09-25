// What Kratos posts to remotehub for each mail (#145, crates/server/src/api/courier.rs).
function(ctx) {
  recipient: ctx.recipient,
  template_type: ctx.template_type,
  template_data: if std.objectHas(ctx, 'template_data') then ctx.template_data else {},
  request_headers: if std.objectHas(ctx, 'request_headers') then ctx.request_headers else {},
  subject: if std.objectHas(ctx, 'subject') then ctx.subject else '',
  body: if std.objectHas(ctx, 'body') then ctx.body else '',
}
