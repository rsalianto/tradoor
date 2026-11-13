// tradoor moved from tradoor.rsalianto.com to tradoorterminal.com. What a browser
// keeps for a site stays with the address it was kept under, so the old address
// sends a visitor on to the same page here, and the app's pages carry the
// browser's id with them — what its drawings and layout on the server are kept
// by — in the fragment, which no server is sent. Here, the id is taken only into
// a browser that has none of its own yet, and the fragment dropped at once. A
// session is not carried: a link anyone could make must not sign a browser into
// someone else's account; signed in there, a reader signs in again here.
//
// A plain script, read before the app: nothing of the app has asked for the
// browser's id yet, which would make one of its own.
'use strict';
(() => {
  const HOME = 'tradoorterminal.com';
  const OWNER = 'tradoor:owner';
  const ID = /^[A-Za-z0-9-]{16,64}$/;
  const MARK = '#moved=';
  const read = (k) => {
    try {
      return localStorage.getItem(k);
    } catch {
      return null;
    }
  };
  const write = (k, v) => {
    try {
      localStorage.setItem(k, v);
    } catch {
      /* storage refused: the browser starts afresh */
    }
  };
  const host = location.hostname;
  if (host === 'tradoor.rsalianto.com' || host === `www.${HOME}`) {
    // The app's pages carry the id; a page of its own (the docs) keeps its anchor.
    const app = !/\.html$/.test(location.pathname);
    const owner = read(OWNER);
    const carry = host !== `www.${HOME}` && app && owner && ID.test(owner) ? `${MARK}${owner}${read('tradoor:session') ? '.s' : ''}` : location.hash;
    location.replace(`https://${HOME}${location.pathname}${location.search}${carry}`);
    return;
  }
  if (location.hash.startsWith(MARK)) {
    const [owner, signedIn] = location.hash.slice(MARK.length).split('.');
    if (owner && ID.test(owner) && !read(OWNER)) write(OWNER, owner);
    if (signedIn === 's') write('tradoor:movedSignedIn', '1');
    history.replaceState(history.state, '', location.pathname + location.search);
  }
})();
