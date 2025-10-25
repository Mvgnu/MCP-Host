'use client';

/* musikconnect:
   purpose: Simple spinning loader for async actions
   inputs: none
   outputs: SVG spinner element
   status: stable
*/

export default function Spinner() {
  return (
    <div className="flex items-center justify-center">
      <div className="h-5 w-5 border-2 border-current border-t-transparent rounded-full animate-spin" />
    </div>
  );
}
