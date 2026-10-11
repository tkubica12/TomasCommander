# Directory aggregate caching

Size and latest descendant-file modification time share one in-memory cache
between the two panes. Complete results are reused only while the same directory
identity is continuously watched. Partial/error results are not reusable hits.
Restart intentionally starts without trusted entries; no disk-cache freshness is
inferred from a directory's mtime.

`src\directory_cache.rs` uses Win32 FindFirstChangeNotificationW with subtree
monitoring for names, sizes, last writes, attributes and security. This coarse
notification is sufficient to invalidate the whole affected aggregate and avoids
maintaining an event-stream buffer or an incremental file index. Directory
identity checks detect replacement/removal of the watched root itself.
Notifications are polled on a background thread every 250 ms, not on the UI.
Windows may delay size/time notifications until writes are flushed.

At most 128 directories retain watches/results, with least-recently-used
eviction. Evicted results are never reused; already displayed numbers remain
snapshots until Refresh or navigation. There is no claim that every directory in
an arbitrarily large listing remains continuously monitored.
Active watched changes trigger an asynchronous panel refresh while retaining
selection, filter and navigation-return history. Queued invalidations are bounded;
overflow requests a conservative full refresh, not trusted stale hits.

Calculation starts after monitoring is established and checks the same generation
again before committing a result. A detected concurrent change permits one
bounded recomputation; ongoing changes produce an explicit error. Both scan
workers use the same cache. They may calculate simultaneously on the first miss,
but warm revisits reuse results without a recursive scan.

Only fixed local drives use live cache hits. Remote/removable paths and failures
to establish monitoring still get an actual calculation, marked with a warning
that live caching is unavailable. Lost monitoring discards entries and is logged.
Refresh and completed/cancelled app operations clear the shared cache and cause
revalidation. Normal desktop FILE-01 containment/race limits still apply.

Validation includes actual Windows notifications for nested edits, create,
rename and delete, exact uncached size/date comparison, warm scan counts, mutation
during calculation, queued UI invalidation, cancellation, watcher bounds and
restart without trusted data. Native UI read-back is recorded separately.
