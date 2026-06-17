//! Builder, setter, and getter macro generators for widget fields.

/// Generates `set_X`, the `X` builder, and `get_X`/`is_X` for a field.
///
/// Use `name as getter:` to override the getter name, e.g. `wrap as wraps: bool`.
#[macro_export]
macro_rules! field {
    ($(#[$attr:meta])* $name:ident as $g:ident : $($rest:tt)+) => {
        $crate::field!(@find [$(#[$attr])*] $name [$g] [] $($rest)+);
    };
    ($(#[$attr:meta])* $name:ident : $($rest:tt)+) => {
        $crate::field!(@find [$(#[$attr])*] $name [] [] $($rest)+);
    };

    (@find [$(#[$attr:meta])*] $name:ident [$($g:ident)?] [$($ty:tt)+] => $head:ident $($rest:tt)*) => {
        $crate::field!(@walk_path [$(#[$attr])*] $name [$($g)?] [$($ty)+] [$head] $($rest)*);
    };
    (@find [$(#[$attr:meta])*] $name:ident [$($g:ident)?] [$($ty:tt)+] ; $cb:ident) => {
        $crate::field!(@type [$(#[$attr])*] $name [$($g)?] [$cb] [$name] $($ty)+);
    };
    (@find [$(#[$attr:meta])*] $name:ident [$($g:ident)?] [$($ty:tt)+]) => {
        $crate::field!(@type [$(#[$attr])*] $name [$($g)?] [] [$name] $($ty)+);
    };

    (@find [$(#[$attr:meta])*] $name:ident [$($g:ident)?] [$($ty:tt)*] $tok:tt $($rest:tt)*) => {
        $crate::field!(@find [$(#[$attr])*] $name [$($g)?] [$($ty)* $tok] $($rest)*);
    };

    (@walk_path [$(#[$attr:meta])*] $name:ident [$($g:ident)?] [$($ty:tt)+] [$($acc:tt)+] ? ; $cb:ident) => {
        $crate::field!(@type_presence [$(#[$attr])*] $name [$($g)?] [$cb] [$($acc)+] $($ty)+);
    };
    (@walk_path [$(#[$attr:meta])*] $name:ident [$($g:ident)?] [$($ty:tt)+] [$($acc:tt)+] ?) => {
        $crate::field!(@type_presence [$(#[$attr])*] $name [$($g)?] [] [$($acc)+] $($ty)+);
    };
    (@walk_path [$(#[$attr:meta])*] $name:ident [$($g:ident)?] [$($ty:tt)+] [$($acc:tt)+] ; $cb:ident) => {
        $crate::field!(@type [$(#[$attr])*] $name [$($g)?] [$cb] [$($acc)+] $($ty)+);
    };
    (@walk_path [$(#[$attr:meta])*] $name:ident [$($g:ident)?] [$($ty:tt)+] [$($acc:tt)+]) => {
        $crate::field!(@type [$(#[$attr])*] $name [$($g)?] [] [$($acc)+] $($ty)+);
    };
    (@walk_path [$(#[$attr:meta])*] $name:ident [$($g:ident)?] [$($ty:tt)+] [$($acc:tt)+] $tok:tt $($rest:tt)*) => {
        $crate::field!(@walk_path [$(#[$attr])*] $name [$($g)?] [$($ty)+] [$($acc)+ $tok] $($rest)*);
    };

    (@type_presence [$(#[$attr:meta])*] $name:ident [$g:ident] [$($dirty:ident)?] [$($p:tt)+] bool) => {
        $crate::field!(@emit_presence [$(#[$attr])*] $name [$g] [$($dirty)?] [$($p)+]);
    };
    (@type_presence [$(#[$attr:meta])*] $name:ident [] [$($dirty:ident)?] [$($p:tt)+] bool) => {
        $crate::paste::paste! {
            $crate::field!(@emit_presence [$(#[$attr])*] $name [[<is_ $name>]] [$($dirty)?] [$($p)+]);
        }
    };

    (@type [$(#[$attr:meta])*] $name:ident [$g:ident] [$($dirty:ident)?] [$($p:tt)+] bool) => {
        $crate::field!(@path [$(#[$attr])*] (bool) $name [$g] [$($dirty)?] $($p)+);
    };
    (@type [$(#[$attr:meta])*] $name:ident [] [$($dirty:ident)?] [$($p:tt)+] bool) => {
        $crate::paste::paste! {
            $crate::field!(@path [$(#[$attr])*] (bool) $name [[<is_ $name>]] [$($dirty)?] $($p)+);
        }
    };
    (@type [$(#[$attr:meta])*] $name:ident [$g:ident] [$($dirty:ident)?] [$($p:tt)+] Option<$t:ty>) => {
        $crate::field!(@path [$(#[$attr])*] (opt $t) $name [$g] [$($dirty)?] $($p)+);
    };
    (@type [$(#[$attr:meta])*] $name:ident [] [$($dirty:ident)?] [$($p:tt)+] Option<$t:ty>) => {
        $crate::paste::paste! {
            $crate::field!(@path [$(#[$attr])*] (opt $t) $name [[<get_ $name>]] [$($dirty)?] $($p)+);
        }
    };
    (@type [$(#[$attr:meta])*] $name:ident [$g:ident] [$($dirty:ident)?] [$($p:tt)+] & Option<$t:ty>) => {
        $crate::field!(@path [$(#[$attr])*] (ref_opt $t) $name [$g] [$($dirty)?] $($p)+);
    };
    (@type [$(#[$attr:meta])*] $name:ident [] [$($dirty:ident)?] [$($p:tt)+] & Option<$t:ty>) => {
        $crate::paste::paste! {
            $crate::field!(@path [$(#[$attr])*] (ref_opt $t) $name [[<get_ $name>]] [$($dirty)?] $($p)+);
        }
    };
    (@type [$(#[$attr:meta])*] $name:ident [$g:ident] [$($dirty:ident)?] [$($p:tt)+] & $lt:lifetime $t:ty) => {
        $crate::field!(@path [$(#[$attr])*] (plain & $lt $t) $name [$g] [$($dirty)?] $($p)+);
    };
    (@type [$(#[$attr:meta])*] $name:ident [] [$($dirty:ident)?] [$($p:tt)+] & $lt:lifetime $t:ty) => {
        $crate::paste::paste! {
            $crate::field!(@path [$(#[$attr])*] (plain & $lt $t) $name [[<get_ $name>]] [$($dirty)?] $($p)+);
        }
    };
    (@type [$(#[$attr:meta])*] $name:ident [$g:ident] [$($dirty:ident)?] [$($p:tt)+] & $t:ty) => {
        $crate::field!(@path [$(#[$attr])*] (ref_t $t) $name [$g] [$($dirty)?] $($p)+);
    };
    (@type [$(#[$attr:meta])*] $name:ident [] [$($dirty:ident)?] [$($p:tt)+] & $t:ty) => {
        $crate::paste::paste! {
            $crate::field!(@path [$(#[$attr])*] (ref_t $t) $name [[<get_ $name>]] [$($dirty)?] $($p)+);
        }
    };
    (@type [$(#[$attr:meta])*] $name:ident [$g:ident] [$($dirty:ident)?] [$($p:tt)+] $t:ty) => {
        $crate::field!(@path [$(#[$attr])*] (plain $t) $name [$g] [$($dirty)?] $($p)+);
    };
    (@type [$(#[$attr:meta])*] $name:ident [] [$($dirty:ident)?] [$($p:tt)+] $t:ty) => {
        $crate::paste::paste! {
            $crate::field!(@path [$(#[$attr])*] (plain $t) $name [[<get_ $name>]] [$($dirty)?] $($p)+);
        }
    };

    (@path [$(#[$attr:meta])*] ($($tag:tt)*) $name:ident [$g:ident] [$($dirty:ident)?] $c:ident $(. $pre:ident)* ? . $first:ident $($rest:tt)*) => {
        $crate::field!(@walk [$(#[$attr])*] ($($tag)*) $name [$g] [$($dirty)?] [$c] [$($pre)*] [] [$first] $($rest)*);
    };
    (@path [$(#[$attr:meta])*] ($($tag:tt)*) $name:ident [$g:ident] [$($dirty:ident)?] $($p:ident).+) => {
        $crate::field!(@emit_plain [$(#[$attr])*] ($($tag)*) $name [$g] [$($dirty)?] [$($p).+]);
    };

    (@walk [$(#[$w:meta])*] ($($tag:tt)*) $name:ident [$g:ident] [$($dirty:ident)?] [$c:ident] [$($pre:ident)*] [$([$($chunk:ident)+])*] [$($cur:ident)+] . $next:ident $($rest:tt)*) => {
        $crate::field!(@walk [$(#[$w])*] ($($tag)*) $name [$g] [$($dirty)?] [$c] [$($pre)*] [$([$($chunk)+])*] [$($cur)+ $next] $($rest)*);
    };
    (@walk [$(#[$w:meta])*] ($($tag:tt)*) $name:ident [$g:ident] [$($dirty:ident)?] [$c:ident] [$($pre:ident)*] [$([$($chunk:ident)+])*] [$($cur:ident)+] ? . $next:ident $($rest:tt)*) => {
        $crate::field!(@walk [$(#[$w])*] ($($tag)*) $name [$g] [$($dirty)?] [$c] [$($pre)*] [$([$($chunk)+])* [$($cur)+]] [$next] $($rest)*);
    };
    (@walk [$(#[$w:meta])*] ($($tag:tt)*) $name:ident [$g:ident] [$($dirty:ident)?] [$c:ident] [$($pre:ident)*] [$([$($mid:ident)+])*] [$($last:ident)+]) => {
        $crate::field!(@emit_lazy [$(#[$w])*] ($($tag)*) $name [$g] [$($dirty)?] [$c] [$($pre)*] [$([$($mid)+])*] [$($last)+]);
    };

    (@emit_presence [$(#[$attr:meta])*] $name:ident [$g:ident] [$($dirty:ident)?] [$($p:tt)+]) => {
        $crate::paste::paste! {
            $(#[$attr])*
            pub fn [<set_ $name>](&mut self, value: bool) {
                if value != self.$($p)+.is_some() {
                    self.$($p)+ = if value {
                        ::core::option::Option::Some(::core::default::Default::default())
                    } else {
                        ::core::option::Option::None
                    };
                    $(Self::$dirty(self);)?
                }
            }
            $(#[$attr])* pub fn $name(mut self: Box<Self>) -> Box<Self> { self.[<set_ $name>](true); self }
            $(#[$attr])* pub fn [<$name _if>](mut self: Box<Self>, value: bool) -> Box<Self> { self.[<set_ $name>](value); self }
        }
        $(#[$attr])* pub fn $g(&self) -> bool { self.$($p)+.is_some() }
    };

    (@emit_plain [$(#[$attr:meta])*] (bool) $name:ident [$g:ident] [$($dirty:ident)?] [$($p:ident).+]) => {
        $crate::field!(@write [$(#[$attr])*] bool, $name, [$($dirty)?], [$($p).+]);
        $(#[$attr])* pub fn $g(&self) -> bool { self.$($p).+ }
    };
    (@emit_plain [$(#[$attr:meta])*] (opt $t:ty) $name:ident [$g:ident] [$($dirty:ident)?] [$($p:ident).+]) => {
        $crate::field!(@write [$(#[$attr])*] Option<$t>, $name, [$($dirty)?], [$($p).+]);
        $(#[$attr])* pub fn $g(&self) -> ::core::option::Option<$t> { self.$($p).+ }
    };
    (@emit_plain [$(#[$attr:meta])*] (ref_opt $t:ty) $name:ident [$g:ident] [$($dirty:ident)?] [$($p:ident).+]) => {
        $crate::field!(@write [$(#[$attr])*] Option<$t>, $name, [$($dirty)?], [$($p).+]);
        $(#[$attr])* pub fn $g(&self) -> ::core::option::Option<&$t> { self.$($p).+.as_ref() }
    };
    (@emit_plain [$(#[$attr:meta])*] (ref_t $t:ty) $name:ident [$g:ident] [$($dirty:ident)?] [$($p:ident).+]) => {
        $crate::field!(@write [$(#[$attr])*] $t, $name, [$($dirty)?], [$($p).+]);
        $(#[$attr])* pub fn $g(&self) -> &$t { &self.$($p).+ }
    };
    (@emit_plain [$(#[$attr:meta])*] (plain $t:ty) $name:ident [$g:ident] [$($dirty:ident)?] [$($p:ident).+]) => {
        $crate::field!(@write [$(#[$attr])*] $t, $name, [$($dirty)?], [$($p).+]);
        $(#[$attr])* pub fn $g(&self) -> $t { self.$($p).+ }
    };

    (@emit_lazy [$(#[$w:meta])*] (bool) $name:ident [$g:ident] [$($dirty:ident)?] [$c:ident] [$($pre:ident)*] [$([$($mid:ident)+])*] [$($last:ident)+]) => {
        $crate::field!(@write [$(#[$w])*] bool, $name, [$($dirty)?],
            [$c $(.$pre)* .get_or_insert_with(::core::default::Default::default) $(.$($mid).+ .get_or_insert_with(::core::default::Default::default))* .$($last).+]);
        $(#[$w])*
        pub fn $g(&self) -> bool {
            (|| -> ::core::option::Option<bool> {
                ::core::option::Option::Some(self.$c $(.$pre)* .as_ref()? $(.$($mid).+ .as_ref()?)* .$($last).+)
            })().unwrap_or(false)
        }
    };
    (@emit_lazy [$(#[$w:meta])*] (opt $t:ty) $name:ident [$g:ident] [$($dirty:ident)?] [$c:ident] [$($pre:ident)*] [$([$($mid:ident)+])*] [$($last:ident)+]) => {
        $crate::field!(@write [$(#[$w])*] Option<$t>, $name, [$($dirty)?],
            [$c $(.$pre)* .get_or_insert_with(::core::default::Default::default) $(.$($mid).+ .get_or_insert_with(::core::default::Default::default))* .$($last).+]);
        $(#[$w])*
        pub fn $g(&self) -> ::core::option::Option<$t> {
            (|| -> ::core::option::Option<$t> {
                self.$c $(.$pre)* .as_ref()? $(.$($mid).+ .as_ref()?)* .$($last).+
            })()
        }
    };
    (@emit_lazy [$(#[$w:meta])*] (ref_opt $t:ty) $name:ident [$g:ident] [$($dirty:ident)?] [$c:ident] [$($pre:ident)*] [$([$($mid:ident)+])*] [$($last:ident)+]) => {
        $crate::field!(@write [$(#[$w])*] Option<$t>, $name, [$($dirty)?],
            [$c $(.$pre)* .get_or_insert_with(::core::default::Default::default) $(.$($mid).+ .get_or_insert_with(::core::default::Default::default))* .$($last).+]);
        $(#[$w])*
        pub fn $g(&self) -> ::core::option::Option<&$t> {
            (|| -> ::core::option::Option<&$t> {
                self.$c $(.$pre)* .as_ref()? $(.$($mid).+ .as_ref()?)* .$($last).+.as_ref()
            })()
        }
    };
    (@emit_lazy [$(#[$w:meta])*] (plain $t:ty) $name:ident [$g:ident] [$($dirty:ident)?] [$c:ident] [$($pre:ident)*] [$([$($mid:ident)+])*] [$($last:ident)+]) => {
        $crate::field!(@write [$(#[$w])*] $t, $name, [$($dirty)?],
            [$c $(.$pre)* .get_or_insert_with(::core::default::Default::default) $(.$($mid).+ .get_or_insert_with(::core::default::Default::default))* .$($last).+]);
        $(#[$w])*
        pub fn $g(&self) -> $t {
            (|| -> ::core::option::Option<$t> {
                ::core::option::Option::Some(self.$c $(.$pre)* .as_ref()? $(.$($mid).+ .as_ref()?)* .$($last).+)
            })().unwrap_or_default()
        }
    };

    (@setter [$(#[$attr:meta])*] $name:ident [$($dirty:ident)?] $sig:ty, [$($w:tt)+]) => {
        $crate::paste::paste! {
            $(#[$attr])*
            pub fn [<set_ $name>](&mut self, value: $sig) {
                let target = &mut self.$($w)+;
                if *target != value {
                    *target = value;
                    $(Self::$dirty(self);)?
                }
            }
        }
    };
    (@write [$(#[$attr:meta])*] bool, $name:ident, [$($dirty:ident)?], [$($w:tt)+]) => {
        $crate::field!(@setter [$(#[$attr])*] $name [$($dirty)?] bool, [$($w)+]);
        $crate::paste::paste! {
            $(#[$attr])*
            pub fn $name(mut self: Box<Self>) -> Box<Self> { self.[<set_ $name>](true); self }
            $(#[$attr])*
            pub fn [<$name _if>](mut self: Box<Self>, value: bool) -> Box<Self> { self.[<set_ $name>](value); self }
        }
    };
    (@write [$(#[$attr:meta])*] Option<$t:ty>, $name:ident, [$($dirty:ident)?], [$($w:tt)+]) => {
        $crate::field!(@setter [$(#[$attr])*] $name [$($dirty)?] Option<$t>, [$($w)+]);
        $crate::paste::paste! {
            $(#[$attr])*
            pub fn $name(mut self: Box<Self>, value: $t) -> Box<Self> { self.[<set_ $name>](Some(value)); self }
            $(#[$attr])*
            pub fn [<$name _opt>](mut self: Box<Self>, value: Option<$t>) -> Box<Self> { self.[<set_ $name>](value); self }
        }
    };
    (@write [$(#[$attr:meta])*] $t:ty, $name:ident, [$($dirty:ident)?], [$($w:tt)+]) => {
        $crate::field!(@setter [$(#[$attr])*] $name [$($dirty)?] $t, [$($w)+]);
        $crate::paste::paste! {
            $(#[$attr])*
            pub fn $name(mut self: Box<Self>, value: $t) -> Box<Self> { self.[<set_ $name>](value); self }
        }
    };
}

/// [`field!`] with `dirty_layout` as the change callback.
#[macro_export]
macro_rules! layout_field {
    ($($t:tt)*) => { $crate::field!($($t)* ; dirty_layout); };
}

/// [`field!`] with `dirty_paint` as the change callback.
#[macro_export]
macro_rules! style_field {
    ($($t:tt)*) => { $crate::field!($($t)* ; dirty_paint); };
}

/// Forwards `set_X`, the `X` builder, and `get_X`/`is_X` to a sub-field's own accessors.
#[macro_export]
macro_rules! delegate_field {
    ($(#[$attr:meta])* $name:ident : bool => $($f:ident).+) => {
        $crate::paste::paste! {
            $(#[$attr])* pub fn [<set_ $name>](&mut self, value: bool) { self.$($f).+.[<set_ $name>](value); }
            $(#[$attr])* pub fn $name(mut self: Box<Self>) -> Box<Self> { self.[<set_ $name>](true); self }
            $(#[$attr])* pub fn [<$name _if>](mut self: Box<Self>, value: bool) -> Box<Self> { self.[<set_ $name>](value); self }
            $(#[$attr])* pub fn [<is_ $name>](&self) -> bool { self.$($f).+.[<is_ $name>]() }
        }
    };
    ($(#[$attr:meta])* $name:ident : Option<$t:ty> => $($f:ident).+) => {
        $crate::paste::paste! {
            $(#[$attr])* pub fn [<set_ $name>](&mut self, value: ::core::option::Option<$t>) { self.$($f).+.[<set_ $name>](value); }
            $(#[$attr])* pub fn $name(mut self: Box<Self>, value: $t) -> Box<Self> { self.[<set_ $name>](::core::option::Option::Some(value)); self }
            $(#[$attr])* pub fn [<$name _opt>](mut self: Box<Self>, value: ::core::option::Option<$t>) -> Box<Self> { self.[<set_ $name>](value); self }
            $(#[$attr])* pub fn [<get_ $name>](&self) -> ::core::option::Option<$t> { self.$($f).+.[<get_ $name>]() }
        }
    };
    ($(#[$attr:meta])* $name:ident : $t:ty => $($f:ident).+) => {
        $crate::paste::paste! {
            $(#[$attr])* pub fn [<set_ $name>](&mut self, value: $t) { self.$($f).+.[<set_ $name>](value); }
            $(#[$attr])* pub fn $name(mut self: Box<Self>, value: $t) -> Box<Self> { self.[<set_ $name>](value); self }
            $(#[$attr])* pub fn [<get_ $name>](&self) -> $t { self.$($f).+.[<get_ $name>]() }
        }
    };
}
