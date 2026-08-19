/*
 * Every off-site destination in one place, so a URL that is still a guess is a
 * guess in exactly one file rather than in three.
 *
 * The GitHub repository is a mirror of the Forgejo instance development
 * actually happens on. It exists so CI can build releases and so people have
 * somewhere to file bugs. See `knowledge-base/product-site.md`.
 */
export const SOURCE = 'https://github.com/grindshell/rhizolog';

/*
 * The README is the documentation until `/docs` exists here. Pointing at it is
 * deliberate rather than a placeholder: it is genuinely the best answer today.
 */
export const README = `${SOURCE}#readme`;
export const ISSUES = `${SOURCE}/issues`;
export const RELEASES = `${SOURCE}/releases`;

export const LICENCE = 'https://www.gnu.org/licenses/agpl-3.0.html';
