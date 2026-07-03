/**
 * Commitlint configuration
 *
 * Enforces Conventional Commits format:
 *   type(scope): subject
 *
 * Types that trigger releases:
 *   feat     → minor version bump
 *   fix      → patch version bump
 *   perf     → patch version bump
 *
 * Types that do NOT trigger releases:
 *   chore, docs, refactor, test, ci, style, build
 *
 * Breaking changes (any type with `!`):
 *   feat!:   → major version bump
 *   fix!:    → major version bump
 */
module.exports = {
  extends: ['@commitlint/config-conventional'],
  rules: {
    'type-enum': [
      2,
      'always',
      [
        'feat',     // New feature (triggers minor release)
        'fix',      // Bug fix (triggers patch release)
        'perf',     // Performance improvement (triggers patch release)
        'refactor', // Code refactoring (no release)
        'docs',     // Documentation (no release)
        'chore',    // Maintenance (no release)
        'test',     // Tests (no release)
        'ci',       // CI/CD changes (no release)
        'style',    // Code style (no release)
        'build',    // Build system (no release)
        'revert',   // Revert (triggers release)
      ],
    ],
    'subject-case': [0], // Allow any case (PT-BR messages)
    'body-max-line-length': [0], // Disable line length limit for body
  },
};
