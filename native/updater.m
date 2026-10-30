// Main-thread-only bridge to Sparkle's standard UI and persisted preferences.
#import <AppKit/AppKit.h>
#import <Sparkle/Sparkle.h>
#include <stdbool.h>

static SPUStandardUpdaterController *controller;
static bool started;

bool codex_updater_start(void) {
    if (controller) return started;
    controller = [[SPUStandardUpdaterController alloc] initWithStartingUpdater:NO
        updaterDelegate:nil userDriverDelegate:nil];
    NSError *error = nil;
    started = [controller.updater startUpdater:&error];
    if (!started) NSLog(@"Could not start updater: %@", error);
    return started;
}

bool codex_updater_available(void) { return started; }
bool codex_updater_can_check(void) { return started && controller.updater.canCheckForUpdates; }
void codex_updater_check(void) {
    if (codex_updater_can_check()) [controller checkForUpdates:nil];
}
bool codex_updater_automatic(void) { return started && controller.updater.automaticallyChecksForUpdates; }
void codex_updater_set_automatic(bool enabled) {
    if (started) controller.updater.automaticallyChecksForUpdates = enabled;
}
