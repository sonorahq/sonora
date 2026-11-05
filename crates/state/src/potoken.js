// Mints YouTube proof-of-origin tokens, inside a real browser engine, and leaves each in a cookie.
//
// The YouTube page carries its own minting client, the one its player asks for every token. That
// client runs Google's BotGuard virtual machine, which attests the browser it is running in, and
// trades the attestation for an integrity token that every later mint reuses. This script waits
// for the client and asks it, the way yt-dlp's `wpc` provider does. Only a real engine passes the
// attestation, which is the whole reason this runs here rather than in the app's own javascript
// interpreter.
//
// The first binding arrives in the query string and every later one through `sonoraPoToken`, which
// the host calls in the page. Each answer goes back in one cookie behind the binding it answers,
// because reading cookies is all the host can do with the page.
(function () {
  "use strict";

  var COOKIE = "SONORA_POT";
  // What the host reads as a failure rather than a token. A token is base64url and never starts
  // with one of these.
  var FAILED = "!";
  // What ends the binding in the cookie. The binding is escaped, so it never holds one, and
  // base64url has none either.
  var SEPARATOR = ":";
  // Where the page keeps its minting client. The names are the player's own obfuscated ones, so
  // `client` falls back to looking for the client wherever it has moved.
  var HOLDER = "havuokmhhs-0";
  var SLOT = "bevasrs";
  // How long the client is waited for, first to appear and then to finish attesting.
  var PATIENCE = 30000;
  var STEP = 200;

  function answer(binding, value) {
    document.cookie =
      COOKIE +
      "=" +
      encodeURIComponent(binding) +
      SEPARATOR +
      value +
      "; path=/; max-age=120";
  }

  function fail(binding, why) {
    answer(
      binding,
      FAILED +
        String(why)
          .slice(0, 180)
          .replace(/[;,\s]+/g, " "),
    );
  }

  function message(error) {
    return error && error.message ? error.message : error;
  }

  // The binding, read the moment this script runs. The page rewrites its own url before it
  // finishes loading and navigates away from it afterwards, so it is read at document start and
  // kept in the tab's own storage for whatever page comes next.
  var wanted = (function () {
    var found = /[?&]binding=([^&]+)/.exec(location.search);
    var value = found ? decodeURIComponent(found[1]) : null;
    try {
      if (value) sessionStorage.setItem(COOKIE, value);
      else value = sessionStorage.getItem(COOKIE);
    } catch (error) {
      // A page that refuses storage still works on its first load.
    }
    return value;
  })();

  // Whether a page before this one already left a token for `binding`. Only a failure is worth
  // replacing.
  function answered(binding) {
    var found = new RegExp("(?:^|; )" + COOKIE + "=([^;]*)").exec(
      document.cookie,
    );
    var prefix = encodeURIComponent(binding) + SEPARATOR;
    return (
      !!found &&
      found[1].indexOf(prefix) === 0 &&
      found[1].charAt(prefix.length) !== FAILED
    );
  }

  // The function that resolves to the page's minting client, or nothing while the player has not
  // set it up yet. It is looked for under its known name first, then on any global that holds an
  // object with a `wpc` function.
  function client() {
    var holder = window[HOLDER];
    if (holder && holder[SLOT] && typeof holder[SLOT].wpc === "function") {
      return holder[SLOT].wpc;
    }
    var names = Object.getOwnPropertyNames(window);
    for (var at = 0; at < names.length; at++) {
      var found = holding(names[at]);
      if (found) return found;
    }
    return null;
  }

  // The `wpc` function one level below the global `name`, if there is one. Frames from other
  // origins throw on every property read, so each read is guarded and they are skipped.
  function holding(name) {
    try {
      var value = window[name];
      if (!value || typeof value !== "object" || value.window === value) return null;
      for (var key in value) {
        var inner = value[key];
        if (inner && typeof inner.wpc === "function") return inner.wpc;
      }
    } catch (error) {
      // A cross-origin frame, or a getter that refuses to be read.
    }
    return null;
  }

  // Mints a token for `binding` and leaves it, or the reason there is none, in the cookie. The
  // client answers `SDF:notready` while it is still attesting, which is waited out.
  function mint(binding) {
    if (!binding) return fail("", "no binding to mint for");
    var started = Date.now();
    (function attempt() {
      var overdue = Date.now() - started > PATIENCE;
      var made = client();
      if (!made) {
        if (overdue) return fail(binding, "the page has no minting client");
        return setTimeout(attempt, STEP);
      }
      made()
        .then(function (minter) {
          return minter.mws({ c: binding, mc: false, me: false });
        })
        .then(function (token) {
          if (!token) throw new Error("the minting client returned nothing");
          answer(binding, token);
        })
        .catch(function (error) {
          if (String(error).indexOf("SDF:notready") >= 0 && !overdue) {
            return setTimeout(attempt, STEP);
          }
          fail(binding, message(error));
        });
    })();
  }

  // How the host asks for every binding after the first. It is not enumerable, so the page's own
  // scripts walking `window` never come across it.
  try {
    Object.defineProperty(window, "sonoraPoToken", { value: mint });
  } catch (error) {
    // The first binding still gets its answer. Later ones time out on the host's side.
  }

  // Nothing waits for the load event. A youtube page in a window no one is looking at can keep
  // loading for a long time, and the minting client is up well before it settles.
  try {
    if (!wanted) fail("", "no binding in the url");
    else if (!answered(wanted)) mint(wanted);
  } catch (error) {
    fail(wanted || "", message(error));
  }
})();
