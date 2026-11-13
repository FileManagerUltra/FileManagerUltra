## ADDED Requirements

### Requirement: Release checks do not execute remote code
Update checks SHALL only fetch bounded release metadata over verified HTTPS.
Downloads SHALL be initiated by the user in a browser at the official repository.
No release-provided URL, command, executable or markup SHALL be executed.

#### Scenario: Release metadata contains an unrelated download URL
- **WHEN** metadata supplies an external or non-HTTPS URL
- **THEN** the application ignores it and uses the locally constructed official release page
