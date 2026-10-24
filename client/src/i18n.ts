// i18n indirection. Before KanKan integration this returns English strings;
// after integration it delegates to KanKan's LanguageContext.
// Every user-facing string must go through t().

const EN: Record<string, string> = {
  'app.title': 'CITY OF TOWERS',
  'app.subtitle': 'a social city-building game',

  'login.username': 'Username',
  'login.password': 'Password',
  'login.submit': 'Log In',
  'login.register': 'Register',

  'lobby.openGames': 'OPEN GAMES',
  'lobby.myGames': 'MY GAMES',
  'lobby.create': 'Create Game',
  'lobby.logout': 'Log Out',
  'lobby.loggedInAs': 'Logged in as: {name}',
  'lobby.col.name': 'Name',
  'lobby.col.host': 'Host',
  'lobby.col.players': 'Players',
  'lobby.col.turn': 'Turn',
  'lobby.col.status': 'Status',
  'lobby.join': 'Join',
  'lobby.enter': 'Enter',
  'lobby.full': '(full)',
  'lobby.inProgress': '(in progress)',
  'lobby.finished': '(finished)',
  'lobby.noOpen': 'No open games. Create one to get started.',
  'lobby.noMine': 'You are not in any game.',

  'create.title': 'Create Game',
  'create.name': 'Game name',
  'create.capacity': 'Capacity',
  'create.turnLength': 'Turn length',
  'create.turnLimit': 'Turn limit',
  'create.turnLimitHint': '0 = unlimited',
  'create.submit': 'Create',
  'create.cancel': 'Cancel',

  'room.waiting': 'WAITING FOR PLAYERS',
  'room.start': 'Start',
  'room.cancel': 'Cancel',
  'room.leave': 'Leave',
  'room.host': '(host)',
  'room.empty': 'empty',
  'room.needTwo': 'Need at least 2 players.',
  'room.cancelConfirm': 'Cancel this game? All players will return to the lobby.',
  'room.cancelGame': 'Cancel Game',
  'room.keepPlaying': 'Keep Playing',
  'room.settings': '{cap} players · {turn} turns · {limit} turn limit',

  'city.build': 'Build',
  'city.tower': 'Tower',
  'city.wall': 'Wall',
  'city.obelisk': 'Obelisk',
  'city.lobby': 'Lobby',
  'city.turn': 'Turn {n} · next in {time}',
  'city.rename': 'Rename city',
  'city.save': 'Save',

  'build.title': 'BUILD',
  'build.hint': 'You may build one structure per turn.',
  'build.cost': 'Cost:',
  'build.requires': 'Requires:',
  'build.none': '—',
  'build.submit': 'Build',
  'build.built': 'Built',
  'build.locked': 'Locked',
  'build.alreadyBuilt': 'You have already built this turn.',
  'build.notEnough': 'Not enough resources.',
  'build.requiresPrefix': 'Requires: {name}.',

  'tower.title': "{name}'s Magic Tower",
  'tower.posts': '{n} / {max} posts',
  'tower.newPost': '+ New Post',
  'tower.noTower': 'This city has no Magic Tower yet.',
  'tower.noPosts': 'No posts yet.',
  'tower.comments': 'Comments',
  'tower.writeComment': 'write a comment…',
  'tower.post': 'Post',
  'tower.guestbook': 'TOWER GUESTBOOK',
  'tower.leaveMessage': 'leave a message…',
  'tower.edit': 'Edit',
  'tower.delete': 'Delete',
  'tower.deleteConfirm': 'Delete this post? This cannot be undone.',
  'tower.posted': 'posted {time}',
  'tower.imageHint': 'Image must be a public URL. Uploads are not supported.',

  'obelisk.title': 'THE OBELISK',
  'obelisk.seen': 'You have seen {n} of {total} fragments.',
  'obelisk.visit': 'Visit the Obelisk',
  'obelisk.allSeen': 'All fragments seen',
  'obelisk.fragments': 'FRAGMENTS',
  'obelisk.solve': 'SOLVE THE RIDDLE',
  'obelisk.submit': 'Submit',
  'obelisk.attempts': 'attempts remaining this minute: {n}',
  'obelisk.messages': 'MESSAGES AT THE OBELISK',
  'obelisk.leaveMessage': 'leave a message…',
  'obelisk.post': 'Post',
  'obelisk.notVisited': 'You have not visited the Obelisk yet.',
  'obelisk.noMessages': 'No messages yet.',
  'obelisk.revealed': 'Fragment {n} of {total} revealed.',
  'obelisk.wrong': 'That is not the answer. Try again.',
  'obelisk.locked': 'Locked until next turn.',
  'obelisk.wait': 'Wait {s}s',

  'over.solved': 'THE RIDDLE IS SOLVED',
  'over.ended': 'THE GAME HAS ENDED',
  'over.winner': '{name} has claimed the Precious',
  'over.noWinner': 'No one solved the riddle.',
  'over.answer': 'The answer was: {answer}',
  'over.standings': 'Final standings',
  'over.return': 'Return to Lobby',
  'over.precious': 'Precious',
  'over.fragments': '{n} fragments',

  'wall.title': 'WALL',
  'wall.noMessages': 'No messages yet. Be the first.',
  'wall.leaveMessage': 'leave a message…',
  'wall.post': 'Post',

  'common.loading': 'Loading…',
  'common.close': 'Close',
  'common.cancel': 'Cancel',
  'common.save': 'Save',
  'common.retry': 'Retry',
  'common.offline': 'offline',
  'common.serverDown': 'Cannot reach the server.'
};

export function t(key: string, vars?: Record<string, string | number>): string {
  let s = EN[key] ?? key;
  if (vars) {
    for (const [k, v] of Object.entries(vars)) {
      s = s.replace(new RegExp('\\{' + k + '\\}', 'g'), String(v));
    }
  }
  return s;
}