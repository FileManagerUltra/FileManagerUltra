## Troubleshooting

| Symptom | Cause / fix |
| :------ | :---------- |
| Dashboard still says "isn't eligible" | Wrong source selected in the dashboard header, or the dashboard's time range predates the first scrape. Widen to 24h. |
| `node_*` absent from Extract metrics | Vector isn't scraping: `ss -tnp '( dport = :9100 )'`, then `journalctl -u vector` and `grep -i prometheus`. |
| `connection refused` in Vector logs | node_exporter not running: `systemctl status prometheus-node-exporter`. |
| `vector validate` fails after editing | Almost always indentation - the new blocks use 2 spaces, like the rest of the file; diff against the `.bak-*`. |
| `vector` won't start after restart | Config error: `vector validate`, restore the backup, redo step 4. |
| Duplicate-looking CPU charts | Both host_metrics and node_exporter are shipping - expected. Disable one (step 8). |
| Vector Permission denied on `/var/log/chrony`, php-fpm logs, Docker socket | Pre-existing, unrelated to this change. Vector runs as user `vector`; fix with group membership / ACLs if those logs matter. |
