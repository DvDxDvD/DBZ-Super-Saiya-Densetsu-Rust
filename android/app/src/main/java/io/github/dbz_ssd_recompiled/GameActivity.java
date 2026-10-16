package io.github.dbz_ssd_recompiled;

import android.os.Build;
import android.os.Bundle;
import android.view.HapticFeedbackConstants;
import android.view.View;
import android.view.Window;
import android.view.WindowManager;

import org.libsdl.app.SDLActivity;

/**
 * The game's activity: SDL's activity, which loads libSDL3.so and libmain.so and calls SDL_main.
 */
public class GameActivity extends SDLActivity {
    /** A light haptic tick under the finger (the touch controls); sent by the native side. */
    private static final int HAPTIC = 0x8002;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        // The screen stays on while the game is in front, and the picture reaches the edges (cutouts included).
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        if (Build.VERSION.SDK_INT >= 28) {
            getWindow().getAttributes().layoutInDisplayCutoutMode =
                    WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
        }
        hideSystemBars();
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemBars();
        }
    }

    /** Immersive full screen: the system bars are hidden and come back with a swipe from the edge. */
    private void hideSystemBars() {
        Window window = getWindow();
        window.getDecorView().setSystemUiVisibility(
                View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                        | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                        | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                        | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                        | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                        | View.SYSTEM_UI_FLAG_FULLSCREEN);
    }

    @Override
    protected boolean onUnhandledMessage(int command, Object param) {
        if (command == HAPTIC) {
            // The system's keyboard-tap feedback: light, and it needs no permission.
            getWindow().getDecorView().performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP);
            return true;
        }
        return false;
    }
}
